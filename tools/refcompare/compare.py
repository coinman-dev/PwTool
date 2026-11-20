#!/usr/bin/env python3
"""Compare the Rust port (pwtech-core) with the original Password Tech core.

Runs the same command lines through the reference harness (original C++
sources under wine, tools/refharness) and the port harness
(crates/pwtech-core/examples/portharness.rs), compares the JSON outputs
field by field and writes tools/refcompare/REPORT.md.

Usage (from anywhere):

    python3 -I tools/refcompare/compare.py [-j JOBS] [--area A,B] [--no-build]
        [--report FILE] [--keep]

Test cases are generated deterministically (fixed random seed), so every run
uses the same inputs. Work files (word lists, databases, ...) are written to
.ref/refcompare/.

Comparison rules: keys present in both outputs are compared exactly; doubles
are compared by their IEEE-754 bits (`X_bits`); `randpool_log` by the
concatenated byte stream; keys that only the reference harness prints and
that the port cannot provide are listed in KNOWN_REF_ONLY, any other key
missing in the port output counts as a mismatch. A case where the port
harness reports the exception "Unsupported" (input the port API cannot
represent, e.g. lone surrogates) is skipped.
"""

import argparse
import base64
import concurrent.futures as cf
import hashlib
import hmac
import json
import os
import random
import re
import shlex
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WORK = os.path.join(ROOT, '.ref', 'refcompare')
FILES = os.path.join(WORK, 'files')
OUTD = os.path.join(WORK, 'out')
REF = os.path.join(ROOT, 'tools', 'refharness', 'run.sh')
PORT = os.path.join(ROOT, 'target', 'release', 'examples', 'portharness')
DEFAULT_REPORT = os.path.join(ROOT, 'tools', 'refcompare', 'REPORT.md')

LOCALES = {1251: 'ru_RU.UTF-8', 1252: 'en_US.UTF-8'}
DEFAULT_ACP = 1251

SEED = 20261120

# keys printed only by the reference harness (internal state of its
# re-implementation of the main window loop, or values the port API does
# not expose); they are not compared
KNOWN_REF_ONLY = {
    'eff_passw_flags', 'eff_passphr_flags', 'passphr_min_len', 'passphr_max_len',
    'retries_before', 'duplicates_before', 'script_security',
    'param_data', 'hashapass_output_len', 'ansi_bytes', 'strings_locale',
}
# per area: additional keys not compared
AREA_IGNORE = {
    'main': {'net_words_len', 'passw_used', 'invalid_spec'},
    'lua': {'net_words_len', 'passw_used', 'invalid_spec'},
}
GLOBAL_IGNORE = {'acp', 'out', 'deterministic'}

SAFE = re.compile(r"^[A-Za-z0-9<>_:+*,.=\-\[\]{}()!#$%&'/;?@^|~]+$")


def s(name, value):
    """String option: plain if harmless, otherwise as UTF-16LE hex."""
    if value and SAFE.match(value):
        return ['--' + name, value]
    return ['--' + name + '-u16hex', value.encode('utf-16-le', 'surrogatepass').hex()]


def b(name, data):
    """Byte option as hex."""
    return ['--' + name + '-hex', data.hex()]


def fpath(name):
    return os.path.join(FILES, name)


class Case:
    counter = 0

    def __init__(self, area, args, glob=(), acp=DEFAULT_ACP, ignore=(), timeout=120, note='', port_args=None):
        Case.counter += 1
        self.id = Case.counter
        self.area = area
        self.args = list(args)
        self.port_args = list(port_args) if port_args is not None else None
        self.glob = list(glob)
        self.acp = acp
        self.ignore = set(ignore)
        self.timeout = timeout
        self.note = note
        self.ref = None
        self.port = None
        self.status = None
        self.diffs = []
        self.ref_only = set()
        self.compared = 0

    def subst(self, args, side):
        out = os.path.join(OUTD, f'{self.id}.{side}')
        return [a.replace('{OUT}', out) for a in args]

    def ref_cmd(self):
        return [REF] + self.glob + self.subst(self.args, 'ref')

    def port_cmd(self):
        args = self.port_args if self.port_args is not None else self.args
        return [PORT, '--acp', str(self.acp)] + self.glob + self.subst(args, 'port')

    def out_file(self, side):
        return os.path.join(OUTD, f'{self.id}.{side}')

    def repro(self):
        r = 'LC_ALL=%s %s' % (LOCALES[self.acp], ' '.join(shlex.quote(x) for x in
                                                           [os.path.relpath(REF, ROOT)] + self.glob + self.subst(self.args, 'ref')))
        args = self.port_args if self.port_args is not None else self.args
        p = ' '.join(shlex.quote(x) for x in ['cargo', 'run', '-q', '--release', '-p', 'pwtech-core', '--example',
                                               'portharness', '--', '--acp', str(self.acp)] + self.glob + self.subst(args, 'port'))
        return r, p


def run_json(cmd, env, timeout):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout, env=env)
    except subprocess.TimeoutExpired:
        return {'ok': False, 'exception': 'TIMEOUT', 'error': 'timeout after %ds' % timeout}
    text = p.stdout.decode('utf-8', 'replace')
    lines = [ln for ln in text.splitlines() if ln.startswith('{')]
    if not lines:
        return {'ok': False, 'exception': 'NO_OUTPUT', 'error': (p.stderr.decode('utf-8', 'replace')[-500:])}
    try:
        return json.loads(lines[-1])
    except ValueError as e:
        return {'ok': False, 'exception': 'BAD_JSON', 'error': str(e)}


def cmp_json(r, p, path, case, ign):
    if isinstance(r, dict) and isinstance(p, dict):
        for k, rv in r.items():
            kp = path + '/' + k
            if k in ign or kp in ign:
                continue
            if k.endswith('_bits') and k[:-5] in r:
                continue
            if k == 'randpool_log':
                if k not in p:
                    case.diffs.append((kp, 'present', 'missing'))
                elif ''.join(rv) != ''.join(p[k]):
                    case.diffs.append((kp, ''.join(rv), ''.join(p[k])))
                else:
                    case.compared += 1
                continue
            bk = k + '_bits'
            if bk in r:
                if bk not in p:
                    case.diffs.append((kp, rv, 'missing in port'))
                elif r[bk] != p[bk]:
                    case.diffs.append((kp, '%r (%s)' % (rv, r[bk]), '%r (%s)' % (p.get(k), p[bk])))
                else:
                    case.compared += 1
                continue
            if k not in p:
                if k in KNOWN_REF_ONLY:
                    case.ref_only.add(k)
                else:
                    case.diffs.append((kp, rv, 'missing in port'))
                continue
            cmp_json(rv, p[k], kp, case, ign)
    elif isinstance(r, list) and isinstance(p, list):
        if len(r) != len(p):
            case.diffs.append((path + '#len', len(r), len(p)))
        for i, (x, y) in enumerate(zip(r, p)):
            cmp_json(x, y, '%s[%d]' % (path, i), case, ign)
    else:
        if r == p and type(r) is type(p):
            case.compared += 1
        else:
            case.diffs.append((path, r, p))


def evaluate(case):
    r, p = case.ref, case.port
    if p.get('exception') == 'Unsupported':
        case.status = 'skipped'
        case.note = (case.note + ' ' if case.note else '') + 'port: ' + p.get('error', '')
        return
    if r.get('ok') != p.get('ok'):
        case.status = 'mismatch'
        case.diffs.append(('/ok', '%s %s: %s' % (r.get('ok'), r.get('exception', ''), r.get('error', '')),
                           '%s %s: %s' % (p.get('ok'), p.get('exception', ''), p.get('error', ''))))
        return
    if not r.get('ok'):
        # both failed: the exception classes are compared where the port
        # can name the original class
        rc, pc = r.get('exception'), p.get('exception')
        comparable = pc not in ('HarnessError', 'panic', 'TIMEOUT') and rc not in ('HarnessError', 'std::exception')
        if rc == 'TIMEOUT' or pc == 'TIMEOUT':
            case.status = 'mismatch' if rc != pc else 'both-timeout'
            if rc != pc:
                case.diffs.append(('/exception', rc, pc))
            return
        if pc == 'panic':
            case.status = 'mismatch'
            case.diffs.append(('/exception', '%s: %s' % (rc, r.get('error')), 'panic: %s' % p.get('error')))
            return
        if comparable and rc != pc:
            case.status = 'mismatch'
            case.diffs.append(('/exception', '%s: %s' % (rc, r.get('error')), '%s: %s' % (pc, p.get('error'))))
            return
        case.status = 'both-error'
        case.note = (case.note + ' ' if case.note else '') + 'ref: %s: %s / port: %s: %s' % (
            rc, r.get('error'), pc, p.get('error'))
        return
    ign = set(GLOBAL_IGNORE) | AREA_IGNORE.get(case.area, set()) | case.ignore
    cmp_json(r, p, '', case, ign)
    case.status = 'mismatch' if case.diffs else 'match'


def run_case(case):
    env = dict(os.environ)
    env['LC_ALL'] = LOCALES[case.acp]
    case.ref = run_json(case.ref_cmd(), env, case.timeout)
    penv = dict(os.environ)
    penv['LC_ALL'] = LOCALES[case.acp]
    case.port = run_json(case.port_cmd(), penv, case.timeout)
    evaluate(case)
    return case


def run_all(cases, jobs, label):
    t0 = time.time()
    done = 0
    with cf.ThreadPoolExecutor(max_workers=jobs) as ex:
        for _ in ex.map(run_case, cases):
            done += 1
            if done % 100 == 0 or done == len(cases):
                print('  %s: %d/%d (%.0fs)' % (label, done, len(cases), time.time() - t0), file=sys.stderr)


# ---------------------------------------------------------------------------
# test files

def write(name, data):
    path = fpath(name)
    with open(path, 'wb') as f:
        f.write(data)
    return path


WORDS = ['apple', 'Banana', 'cherry', 'date', 'Elder', 'fig', 'grape', 'hazel', 'iris', 'jade', 'kiwi',
         'lemon', 'mango', 'nut', 'olive', 'pear', 'quince', 'rose', 'sage', 'thyme', 'umber', 'violet',
         'walnut', 'xenon', 'yam', 'zest', 'äpfel', 'жук', 'Ёлка', 'smile😀', 'ab', 'a', 'Über', 'naïve',
         'CAPS', 'MiXeD', 'apple', 'APPLE', 'x' * 30, 'longwordwithmanyletters']


def make_files():
    os.makedirs(FILES, exist_ok=True)
    os.makedirs(OUTD, exist_ok=True)
    text = '\n'.join(WORDS) + '\n'
    write('wl_utf8.txt', text.encode('utf-8'))
    write('wl_utf8bom.txt', b'\xef\xbb\xbf' + ('\r\n'.join(WORDS)).encode('utf-8'))
    write('wl_utf16.txt', b'\xff\xfe' + ('\t'.join(WORDS)).encode('utf-16-le'))
    write('wl_utf16be.txt', b'\xfe\xff' + ('  '.join(WORDS) + ' ').encode('utf-16-be'))
    ansi_words = [w for w in WORDS if all(ord(c) < 0x10000 for c in w)]
    write('wl_ansi1251.txt', ('\n'.join(ansi_words)).encode('cp1251', 'replace'))
    write('wl_ansi1252.txt', ('\n'.join(ansi_words)).encode('cp1252', 'replace'))
    write('wl_one.txt', b'single\n')
    write('wl_two.txt', b'one two')
    write('wl_dups.txt', b'Abc abc ABC aBc def DEF\n')
    write('wl_ws.txt', b' \t\r\n  \n\x01word1\x02 \x7fword2\n\x0bword3\x0c\n')
    write('wl_long.txt', ('short ' + 'L' * 1100 + ' other\n').encode('utf-8'))
    write('wl_long1022.txt', ('aa ' + 'M' * 1022 + '\nbb').encode('utf-8'))
    write('wl_long1023.txt', ('aa ' + 'N' * 1023 + '\nbb').encode('utf-8'))
    write('wl_long1023eof.txt', ('aa bb ' + 'O' * 1023).encode('utf-8'))
    write('wl_long_utf16.txt', b'\xff\xfe' + ('short ' + 'P' * 1100 + ' other').encode('utf-16-le'))
    write('wl_badutf8.txt', b'good \xff\xfebad \xc3( fine\n')
    write('wl_odd_utf16.txt', b'\xff\xfea\x00b')
    write('wl_empty.txt', b'')
    rng = random.Random(SEED + 1)
    big = sorted({''.join(rng.choice('abcdefghijklmnopqrstuvwxyz') for _ in range(rng.randint(1, 12)))
                  for _ in range(3000)})
    write('wl_big.txt', ('\n'.join(big)).encode('utf-8'))
    # trigram sources
    write('tri_src_utf8.txt', ('\n'.join(big[:2000]) + '\nÄpfel Straße naïve\n').encode('utf-8'))
    write('tri_src_utf16.txt', b'\xff\xfe' + ('\r\n'.join(big[:500] + ['MiXeDCaSe', 'Zebra'])).encode('utf-16-le'))
    write('tri_src_small.txt', b'abc def\n')
    write('tri_src_none.txt', b'12 34 !!\n')
    write('tri_bad_size.bin', b'\0' * 100)
    # key files
    write('key32.bin', bytes(range(32)))
    write('key64hex_upper.txt', bytes(range(100, 132)).hex().upper().encode())
    write('key64hex_lower.txt', bytes(range(200, 232)).hex().encode())
    write('key64hex_mixed.txt', b'aB' * 32)
    write('key64_nonhex.txt', b'g' + b'0' * 63)
    write('key63.bin', bytes(range(63)))
    write('key65.bin', bytes(range(65)))
    write('key1.bin', b'\x01')
    write('key_empty.bin', b'')
    write('key64hex_nl.txt', bytes(range(32)).hex().encode() + b'\n')
    write('key_big.bin', bytes(rng.randrange(256) for _ in range(100000)))
    write('key_text.txt', 'Schlüssel ключ 😀'.encode('utf-8'))
    # lua scripts
    write('lua_standalone.lua', b'''script_flags = 1
function init(n, dest, genflags, adv, nchars, nwords, fmt)
  g_init = string.format("%d/%d/%d/%d/%d/%d/%s", n, dest, genflags, adv, nchars, nwords, fmt)
end
function generate(num)
  local p, s = pwtech_password(10)
  local ph, s2 = pwtech_phonetic(6, 256)
  local pp, s3 = pwtech_passphrase(2, "x9", 1)
  local f, s4 = pwtech_format("4d-3a")
  local w = pwtech_word()
  local w1 = pwtech_word(1)
  local r = pwtech_random(1, 6)
  local r2 = pwtech_random(1000)
  local d = pwtech_random()
  return p .. "|" .. ph .. "|" .. pp .. "|" .. f .. "|" .. w .. "|" .. w1 .. "|" .. r .. "|" .. r2 .. "|" ..
    string.format("%.17g", d) .. "|" .. pwtech_numwords() .. "|" .. g_init, s + s2 + s3 + s4
end
''')
    write('lua_modify.lua', b'''function generate(num, passw, sec)
  if passw == nil then return "", 0 end
  local t = {}
  for c in passw:gmatch(".") do table.insert(t, c) end
  local out = {}
  for i = #t, 1, -1 do table.insert(out, t[i]) end
  return table.concat(out) .. num, sec - 1
end
''')
    write('lua_empty.lua', b'''function generate(num, passw, sec)
  return "", 12345
end
''')
    write('lua_unicode.lua', b'''function generate(num, passw, sec)
  return "\xc3\xa4\xd0\xb6\xf0\x9f\x98\x80" .. (passw or "-"), -5
end
''')
    write('lua_bigsec.lua', b'''function generate(num, passw, sec)
  return (passw or "") .. "!", 1e9
end
''')
    write('lua_error.lua', b'''function generate(num, passw, sec)
  error("boom")
end
''')
    write('lua_nogen.lua', b'''x = 1
''')


# ---------------------------------------------------------------------------
# case generators

def cases_prng(rng):
    out = []
    seeds = [b'', b'\x01', b'test', bytes(range(32)), bytes(range(100))]
    for sd in seeds:
        out.append(Case('prng', ['prng-aesctr'] + b('seed', sd) + ['--bytes', '100']))
    for key, param in [(b'k', b''), (bytes(32), b''), (b'key', b'param'), (bytes(range(64)), b'\x00' * 5)]:
        out.append(Case('prng', ['prng-aesctr'] + b('key', key) + (b('param', param) if param else []) +
                        ['--bytes', '64', '--numrange', '1000', '--count', '20']))
    for n in [0, 1, 15, 16, 17, 63, 64, 65, 129, 1000]:
        out.append(Case('prng', ['prng-aesctr'] + b('seed', b'n%d' % n) + ['--bytes', str(n), '--after-bytes', '7']))
    for m in [1, 2, 3, 7, 100, 128, 255, 256, 257, 1000, 4097, 65535, 65536, 65537, 100000, 2 ** 31, 2 ** 32 - 1,
              3 * 10 ** 9]:
        out.append(Case('prng', ['prng-aesctr'] + b('seed', b'r') + ['--numrange', str(m), '--count', '40',
                                                                    '--after-bytes', '8']))
    # crossing the re-key boundary (65536 blocks)
    for skip in [1048576 - 10, 1048576, 2097152 + 3]:
        out.append(Case('prng', ['prng-aesctr'] + b('seed', b'skip') + ['--skip', str(skip), '--bytes', '64']))
    out.append(Case('prng', ['prng-aesctr'] + b('seed', b'big') + ['--bytes', '3000000', '--digest']))
    out.append(Case('prng', ['prng-aesctr'] + b('key', b'k') + ['--param-utf16', 'pä', '--bytes', '32']))
    return out


def cases_hash(rng):
    out = []
    for pw, salt, it in [(b'secret', b'\x00\x11\x22\x33', 8192), (b'', b'', 1), (b'p', b'', 2),
                         ('пароль'.encode('utf-16-le'), b'salt' * 4, 1000), (bytes(range(200)), bytes(64), 3),
                         (b'x' * 65, b'y', 100)]:
        out.append(Case('hash', ['pbkdf2'] + b('passw', pw) + (b('salt', salt) if salt else []) + ['--iter', str(it)]))
    out.append(Case('hash', ['pbkdf2', '--passw-utf16', 'secret', '--salt-hex', '00112233']))
    keys = [b'', b'k', bytes(range(64)), bytes(range(65)), bytes(range(128)), bytes(range(129)), bytes(200)]
    datas = [b'', b'd', b'The quick brown fox', bytes(range(256)) * 3]
    for alg in ['hmac-sha256', 'hmac-sha512', 'hmac-sha1']:
        for k in keys:
            d = rng.choice(datas)
            out.append(Case('hash', [alg, '--key-hex', k.hex()] + (b('data', d) if d else [])))
    for alg in ['sha256', 'sha512', 'sha1']:
        for n in [0, 1, 55, 56, 63, 64, 111, 112, 127, 128, 1000]:
            d = bytes(rng.randrange(256) for _ in range(n))
            out.append(Case('hash', [alg] + (b('data', d) if d else [])))
    out.append(Case('hash', ['sha256', '--file', fpath('key_big.bin')]))
    out.append(Case('hash', ['sha512', '--data-utf8', 'äöü😀']))
    return out


def cases_chacha(rng):
    out = []
    for klen in [16, 32]:
        for rounds in [0, 1, 5, 6, 7, 8, 12, 13, 20, 21, 32]:
            key = bytes(rng.randrange(256) for _ in range(klen))
            iv = bytes(rng.randrange(256) for _ in range(8))
            out.append(Case('chacha', ['chacha'] + b('key', key) + b('iv', iv) + ['--rounds', str(rounds),
                                                                                  '--bytes', '130']))
    for n in [0, 1, 63, 64, 65, 200]:
        out.append(Case('chacha', ['chacha'] + b('key', bytes(32)) + b('iv', bytes(8)) + ['--bytes', str(n)]))
    for ctr in [bytes(8), b'\xff\xff\xff\xff\x00\x00\x00\x00', b'\xfe\xff\xff\xff\xff\xff\xff\xff', b'\x01' * 8]:
        out.append(Case('chacha', ['chacha'] + b('key', bytes(range(32))) + b('iv', bytes(range(8))) + b('ctr', ctr) +
                        ['--bytes', '200']))
    for chunks in ['10', '64,1,63', '1,1,1', '100', '0,5', '63,64,65']:
        out.append(Case('chacha', ['chacha'] + b('key', bytes(range(32))) + b('iv', bytes(8)) +
                        ['--bytes', '300', '--chunks', chunks]))
        data = bytes(rng.randrange(256) for _ in range(150))
        out.append(Case('chacha', ['chacha'] + b('key', bytes(range(16))) + b('iv', bytes(8)) + b('data', data) +
                        ['--chunks', chunks]))
    return out


CS_TOKENS = ['<AZ>', '<az>', '<09>', '<Hex>', '<HEX>', '<hex>', '<base64>', '<b64>', '<easytoread>', '<etr>',
             '<symbols>', '<sym>', '<brackets>', '<brac>', '<punctuation>', '<punct>', '<highansi>', '<high>',
             '<ad>', '<AF>', '<!/>', '<аж>', '<za>', '<aa>', '<a😀>', '<ab', '<<abc>>', '<<äöü>>', '<<😀😁😂>>',
             '<<a>>', '<<>>', '<<x>>>', '<<<AZ>>>', '<<a<b>>', '<<ab>c>>', 'abc', 'xyz', 'ä', 'ж', '😀', '€', '0123',
             ' ', '\t', '-', '<', '>', '>>', ':', '[', ']', ':3', 'AAA', 'ß', 'ÿ', 'Ā']
CS_FREQ = [':1', ':2', ':3+', ':0', ':10', ':123456', ':+', ':1+', ':02', ':99999+']
AMBIGS = ['', '0O1l', '0O 1lI', 'B8 G6 I1l| 0OQD S5 Z2', 'оo ес аa', '0O 1', 'ab', 'a b c', ' 0O', '0O1l ', 'äa üu']
SYMBOLS = ['', '!@#', 'äö€', '😀😁', 'aa!!', '!', '<>']


def random_charset(rng):
    toks = []
    if rng.random() < 0.15:
        toks.append(rng.choice(['[comment]', '[]', '[x', '[ä]']))
    for _ in range(rng.randint(1, 5)):
        t = rng.choice(CS_TOKENS)
        toks.append(t)
        if t.endswith('>') and rng.random() < 0.4:
            toks.append(rng.choice(CS_FREQ))
    return ''.join(toks)


FIXED_CHARSETS = ['<AZ><az><09>', '<AZ>:2<az>:2<09>:2', '<AZ>:1+<az><09>:3', '<phonetic>', '<phoneticu>',
                  '<phoneticx>', '[x]<phonetic>', '[comment] <phoneticx>', 'a', 'ab', '', '<<ab>>', 'abcabc',
                  '<AZ:2>', '<<abc>>:2<09>', '<sym>:5<etr>', '<highansi>', '<high>:2<az>', '<AZ><az><09><sym>',
                  '<09>:1<az>:1<AZ>:1', '<09>:3<AZ>:3<az>:3<sym>:3', '<az>:2+<09>:2+', '<az>:0<09>', 'äöü😀😁ж',
                  '<<äöü😀>>:2<az>', '<аж>:2<09>', 'a b\tc', ' ', '  ', '<ab', '<<<a>>', '<AZ>:1<az>:1',
                  '<az>:5<AZ>:5<09>:5', '<09><az>:3<AZ>:3', 'abc<09>:2', '<09>:1abc', 'ab<AZ>:1']


def gen_seed_args(rng):
    c = rng.random()
    if c < 0.6:
        return ['--seed-passw', rng.choice(['test', 'x', 'seed1', 'Pässwörd'])] if c < 0.5 else s('seed-passw', 'пароль😀')
    if c < 0.8:
        return b('seed', bytes(rng.randrange(256) for _ in range(rng.randint(0, 20))))
    return b('key', b'key') + b('param', b'p')


def setup_args(rng, cs=None):
    args = []
    if cs is None:
        cs = random_charset(rng)
    args += s('charset', cs)
    if rng.random() < 0.3:
        args.append('--exclude-ambig')
        a = rng.choice(AMBIGS)
        if a:
            args += s('ambig', a)
    elif rng.random() < 0.1:
        args += s('ambig', rng.choice(AMBIGS[1:]))
    if rng.random() < 0.25:
        sy = rng.choice(SYMBOLS)
        if sy:
            args += s('symbols', sy)
    if rng.random() < 0.3:
        args.append('--include-each-subset')
    if rng.random() < 0.3:
        args.append('--determine-subsets')
    return args


def cases_charsets(rng):
    out = []
    for cs in FIXED_CHARSETS:
        out.append(Case('charsets', ['gen', '--seed-passw', 'cs'] + s('charset', cs) + ['--length', '12', '--count', '3']))
        out.append(Case('charsets', ['gen', '--seed-passw', 'cs', '--exclude-ambig', '--include-each-subset'] +
                        s('charset', cs) + ['--length', '12', '--count', '2']))
    for _ in range(220):
        args = ['gen'] + gen_seed_args(rng) + setup_args(rng)
        args += ['--length', str(rng.choice([1, 2, 3, 8, 16, 40])), '--count', '3']
        if rng.random() < 0.3:
            args += ['--flags', str(rng.choice([4, 8, 16, 32, 64, 4 | 8 | 16 | 32 | 64, 2, 0x200, 0x800]))]
        out.append(Case('charsets', args))
    for cs in ['<highansi>', '<high>:3<az>', '<AZ><highansi>']:
        out.append(Case('charsets', ['gen', '--seed-passw', 'hi'] + s('charset', cs) + ['--length', '20'],
                        acp=1252, note='CP 1252'))
    # charsets command: perm entropy, words
    for n, m in [(62, 16), (10, 10), (10, 11), (1, 1), (0, 0), (8192, 4), (100, 0), (2, 1), (1000000, 3), (95, 20)]:
        out.append(Case('charsets', ['charsets', '--perm-set-size', str(n), '--perm-samples', str(m)]))
    for i in [0, 1, 8191, 4000]:
        out.append(Case('charsets', ['charsets', '--word-index', str(i)]))
    for wl in ['wl_utf8.txt', 'wl_utf16.txt']:
        for i in [0, 5, 29]:
            out.append(Case('charsets', ['charsets', '--wordlist', fpath(wl), '--word-index', str(i)]))
    for a in AMBIGS[1:]:
        for sy in ['', 'äö€', '😀😁']:
            out.append(Case('charsets', ['charsets', '--exclude-ambig', '--determine-subsets'] + s('ambig', a) +
                            (s('symbols', sy) if sy else []) + s('charset', '<etr><sym><AZ>:2<09>:1')))
    return out


PW_CHARSETS = ['<AZ><az><09>', '<AZ><az><09><sym>', '<az>', 'ab', 'abc', '<AZ>:2<az>:3+<09>', 'a b\tc', ' ab ',
               '<etr>', 'äöü😀😁ж', '<09>', '<highansi>', '<AZ><az><09><sym><highansi>', '<AZ>:1<az>:1<09>:1<sym>:1',
               ' \t', 'aA1!', '<sym>', '<<😀😁>>:1<az>']


def cases_password(rng):
    out = []
    flagbits = [0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x200, 0x400, 0x800]
    for _ in range(170):
        cs = rng.choice(PW_CHARSETS)
        flags = 0
        for fb in flagbits:
            if rng.random() < 0.25:
                flags |= fb
        length = rng.choice([1, 2, 3, 5, 10, 16, 32, 64, 130, 300])
        args = ['gen'] + gen_seed_args(rng) + s('charset', cs) + ['--length', str(length), '--count', '3',
                                                                    '--flags', str(flags)]
        if flags & 0x4 or rng.random() < 0.2:
            args.append('--determine-subsets')
        if rng.random() < 0.2:
            args.append('--exclude-ambig')
        if rng.random() < 0.15:
            args.append('--include-each-subset')
        out.append(Case('password', args, timeout=60))
    for length in [0, -1, 1, 62, 63, 100]:
        out.append(Case('password', ['gen', '--seed-passw', 'eco', '--length', str(length), '--flags', '512']))
    return out


def cases_phonetic(rng):
    out = []
    flagsets = [0, 1, 2, 0x80, 0x100, 0x180, 0x81, 0x101, 0x800, 0x200, 3, 0x102]
    for fl in flagsets:
        for length in [1, 2, 3, 4, 10, 25]:
            out.append(Case('phonetic', ['gen', '--seed-passw', 'ph%d' % fl, '--mode', 'phonetic', '--length',
                                         str(length), '--flags', str(fl), '--count', '3']))
    for length in [0, -5, 200]:
        out.append(Case('phonetic', ['gen', '--seed-hex', '01', '--mode', 'phonetic', '--length', str(length)]))
    for cs in ['<phonetic>', '<phoneticu>', '<phoneticx>']:
        out.append(Case('phonetic', ['gen', '--seed-passw', 'pc'] + s('charset', cs) +
                        ['--mode', 'phonetic', '--length', '12', '--count', '3']))
    for tri in ['tri_bad_size.bin', 'missing.bin']:
        out.append(Case('phonetic', ['gen', '--seed-passw', 'pt', '--trigram', fpath(tri), '--mode', 'phonetic',
                                     '--length', '12']))
    return out


WORDLISTS = ['wl_utf8.txt', 'wl_utf8bom.txt', 'wl_utf16.txt', 'wl_utf16be.txt', 'wl_one.txt', 'wl_two.txt',
             'wl_dups.txt', 'wl_ws.txt', 'wl_long.txt', 'wl_long1022.txt', 'wl_long1023.txt', 'wl_long1023eof.txt',
             'wl_long_utf16.txt', 'wl_badutf8.txt', 'wl_odd_utf16.txt', 'wl_empty.txt', 'wl_big.txt', 'missing.txt']
SEPS = ['', '-', '_', ' ', 'ä', '😀', '--', '..', ' + ']


def cases_passphrase(rng):
    out = []
    for wl in WORDLISTS:
        for opts in [[], ['--lowercase-words'], ['--min-word-len', '4', '--max-word-len', '6'],
                     ['--min-word-len', '0', '--max-word-len', '0'], ['--min-word-len', '30', '--max-word-len', '3']]:
            out.append(Case('passphrase', ['gen', '--seed-passw', 'wl', '--wordlist', fpath(wl)] + opts +
                            ['--mode', 'passphrase', '--words', '3', '--count', '2']))
    for enc, acp, wl in [('0', 1251, 'wl_ansi1251.txt'), ('0', 1252, 'wl_ansi1252.txt'), ('1', 1251, 'wl_utf8.txt'),
                         ('2', 1251, 'wl_utf8.txt'), ('0', 1251, 'wl_utf8bom.txt'), ('1', 1251, 'wl_utf16.txt')]:
        out.append(Case('passphrase', ['gen', '--seed-passw', 'enc', '--wordlist', fpath(wl), '--mode', 'passphrase',
                                       '--words', '4', '--count', '2'], glob=['--file-encoding', enc], acp=acp))
    for _ in range(110):
        args = ['gen'] + gen_seed_args(rng)
        sep = rng.choice(SEPS)
        if sep:
            args += s('word-sep', sep)
        wcs = rng.choice(SEPS)
        if wcs:
            args += s('word-char-sep', wcs)
        if rng.random() < 0.6:
            args += ['--wordlist', fpath(rng.choice(['wl_utf8.txt', 'wl_utf16.txt', 'wl_big.txt', 'wl_dups.txt']))]
            if rng.random() < 0.3:
                args.append('--lowercase-words')
        flags = 0
        for fb in [1, 2, 4, 8, 0x10, 0x20]:
            if rng.random() < 0.3:
                flags |= fb
        words = rng.choice([0, 1, 2, 3, 4, 8, 20])
        args += ['--mode', 'passphrase', '--words', str(words), '--flags', str(flags), '--count', '3']
        ch = rng.choice(['', 'abc', 'X1!', 'ä😀', 'a', '0123456789'])
        if ch:
            args += s('chars', ch)
        out.append(Case('passphrase', args, timeout=60))
    return out


FMT_SPECS = list('xaAUEdhHlLuvVZcCzpbsSyqQrWwP')
FMT_INVALID = list('efgijkmnotBDFGIJKMNORTXY')
FMT_NUMS = ['', '', '', '1', '2', '5', '10', '0', '00', '3-6', '6-3', '1-1', '12345', '123456', '-3', '5-', '2-0']
FMT_LIT = ['-', '.', '!', ' ', 'ä', '😀', '""', '"lit"', '"x y"', '"', '\t', '#', '+', '^', 'ж']


def random_format(rng, depth=0):
    parts = []
    for _ in range(rng.randint(1, 5)):
        c = rng.random()
        if c < 0.5:
            parts.append(('*' if rng.random() < 0.15 else '') + rng.choice(FMT_NUMS) + rng.choice(FMT_SPECS))
        elif c < 0.6:
            parts.append(rng.choice(FMT_NUMS) + rng.choice(FMT_INVALID))
        elif c < 0.72:
            parts.append(rng.choice(FMT_LIT))
        elif c < 0.82 and depth < 5:
            parts.append(rng.choice(['', '2', '3', '0', '1']) + '[' + random_format(rng, depth + 1) + ']' +
                         ('' if rng.random() < 0.8 else rng.choice(['', ']'])))
        elif c < 0.92 and depth < 5:
            parts.append(rng.choice(['', '2', '3', '0', '5']) + '{' + random_format(rng, depth + 1) + '}')
        else:
            parts.append(rng.choice(FMT_NUMS) + rng.choice(['<<abc>>', '<<<09>>>', '<<äö😀>>', '<<a>>', '<<>>',
                                                             '<<<AZ>:2<az>>>', '<<ab', '<<a>b>>']))
    return ''.join(parts)


FIXED_FORMATS = ['xxxx', '8x', '16a', '5-10d', '10-5d', '*8d', '*d', '*12x', '*W', '3W', '3w', '*3w', '[ab]3', '3[ab]',
                 '3[2[xd]-]', '4[3[2[ab]]]', '2[2[2[2[2[a]]]]]', '{4d4a}', '2{4d4a}', '{ab}', '{a}', '3<<abc>>',
                 '<<abc>>', '8<<<AZ><09>>>', '"verbatim"', '"a""b"', '""', 'P', 'Px', '3P', 'P"-"P', 'qQr', '5q8Q3r',
                 '12345d', '123456d', '20000d', '99999x', '[x]99999', '0d', '00d', 'e', 'efg', '5e3f', 'ä3d', 'Ÿ',
                 '[comment]8x', '[unterminated', '[]d', '-', '5', '5-', 'd5', '*', '{', '}', '[', ']', '>>', '<<',
                 '<<<<a>>>>', '2<<ab>>3<<cd>>', '*5<<abc>>', '*20<<abc>>', 'x{3[a]}', ' 4d ', '  ', '\t4d\t',
                 's8S', 'y4', 'E10', 'zZcCvV', 'bpBP']


def cases_format(rng):
    out = []
    setups = [[], ['--exclude-ambig'], s('charset', 'abc😀') + s('symbols', 'äö€'),
              ['--wordlist', fpath('wl_utf8.txt')] + s('word-sep', '_'), ['--exclude-ambig'] + s('ambig', '0O 1lI'),
              ['--trigram', fpath('tri_bad_size.bin')]]
    passws = [None, 'secret', '', 'äö😀', 'x' * 100]
    for f in FIXED_FORMATS:
        for flags in [0, 3]:
            pw = rng.choice(passws)
            args = ['gen', '--seed-passw', 'fmt', '--mode', 'format'] + s('format', f) + ['--flags', str(flags),
                                                                                       '--count', '2']
            if pw is not None:
                args += s('format-passw', pw)
            out.append(Case('format', args + rng.choice(setups)))
    for _ in range(200):
        f = random_format(rng)
        if rng.random() < 0.1:
            f = '[' + rng.choice(['c', 'comment', '']) + ']' + f
        args = ['gen'] + gen_seed_args(rng) + ['--mode', 'format'] + s('format', f) + \
            ['--flags', str(rng.choice([0, 0, 1, 2, 3])), '--count', '3']
        pw = rng.choice(passws)
        if pw is not None:
            args += s('format-passw', pw)
        args += rng.choice(setups)
        out.append(Case('format', args, timeout=60))
    return out


PASSPHR_LENS = ['>=20', '10-30', '*25', '<15', '<=40*', '>10', 'abc', '5', '20-10', '-5', '>', '*', '0', '30*-40',
                '1-1000']
SCRIPTS = ['alt_phonetic.lua', 'leetspeak.lua', 'passphrase_mixedcase.lua', 'permutation_numbers.lua',
           'sorted_words.lua', 'spelling_alphabet.lua']


def main_args(rng, allow_script=True):
    args = ['gen', '--mode', 'main'] + gen_seed_args(rng)
    length = rng.choice([0, 0, 5, 8, 12, 20])
    words = rng.choice([0, 0, 1, 2, 3, 5])
    fmt = rng.choice(['', '', '', 'P', '"<"P">"', '3dP', '{P3d}', '4x', '5-8d"-"P', '2[x]-P-2[d]', 'PP', '*4d'])
    if length == 0 and words == 0 and not fmt:
        length = 10
    cs = rng.choice(['<AZ><az><09>', '<AZ><az><09><sym>', 'abc', '<phoneticx>', '<phonetic>', '<AZ>:2<az>:2<09>:2',
                     'a b\tc d', '<etr>', 'äöü😀', '<09>'])
    args += s('charset', cs)
    if rng.random() < 0.4:
        args += ['--wordlist', fpath(rng.choice(['wl_utf8.txt', 'wl_big.txt', 'wl_two.txt', 'wl_dups.txt']))]
    if rng.random() < 0.3:
        args += s('word-sep', rng.choice(['-', '_', 'ä']))
    pf = 0
    for fb in [0x2, 0x4, 0x8, 0x10, 0x20, 0x40, 0x200, 0x800]:
        if rng.random() < 0.2:
            pf |= fb
    phf = 0
    for fb in [0x2, 0x4, 0x8, 0x10, 0x20]:
        if rng.random() < 0.25:
            phf |= fb
    ff = (1 if pf & 0x2 else 0) | (2 if pf & 0x800 else 0)
    if length == 0:
        # flags shared with the format: take them from the format side
        if rng.random() < 0.3:
            ff = rng.choice([1, 2, 3])
        pf = (0x2 if ff & 1 else 0) | (0x800 if ff & 2 else 0)
    if pf & 0x4:
        args.append('--determine-subsets')
    args += ['--length', str(length), '--words', str(words), '--passw-flags', str(pf), '--passphr-flags', str(phf),
             '--format-flags', str(ff)]
    if fmt:
        args += s('format', fmt)
    if rng.random() < 0.3:
        args.append('--combine')
    if rng.random() < 0.3:
        args.append('--first-char-not-lc')
    if words and rng.random() < 0.3:
        args += s('passphr-len', rng.choice(PASSPHR_LENS))
    count = rng.choice([1, 1, 2, 5, 10])
    if rng.random() < 0.3:
        args.append('--exclude-duplicates')
    if rng.random() < 0.3:
        args.append('--check-each')
    args += ['--count', str(count)]
    return args


def cases_main(rng):
    out = []
    for _ in range(150):
        out.append(Case('main', main_args(rng), timeout=90))
    # targeted cases
    base = ['gen', '--mode', 'main', '--seed-passw', 'm']
    out += [
        Case('main', base + ['--length', '3', '--count', '10', '--exclude-duplicates'] + s('charset', 'abc')),
        Case('main', base + ['--length', '200', '--passw-flags', '512', '--count', '2'] + s('charset', '<AZ><az>')),
        Case('main', base + ['--length', '130', '--count', '2'] + s('charset', '<AZ><az><09><sym><highansi>')),
        Case('main', base + ['--length', '10', '--passw-flags', '2048', '--count', '5', '--check-each'] +
             s('charset', ' a b ')),
        Case('main', base + ['--words', '10', '--passphr-flags', '32', '--count', '2', '--wordlist',
                             fpath('wl_two.txt')]),
        Case('main', base + ['--words', '3', '--count', '4', '--check-each', '--first-char-not-lc'] +
             s('passphr-len', '10-20')),
        Case('main', base + ['--length', '6', '--words', '2', '--combine', '--first-char-not-lc', '--count', '3']),
        Case('main', base + ['--format', '4d', '--length', '8', '--count', '3', '--check-each']),
        Case('main', base + ['--format', 'P', '--count', '2']),
        Case('main', base + ['--length', '12', '--count', '3'] + s('charset', '<phoneticu>')),
        Case('main', base + ['--length', '12', '--count', '3', '--first-char-not-lc'] + s('charset', '<phonetic>')),
        Case('main', base + ['--length', '40', '--count', '2'] + s('charset', '<AZ>:30<09>:30')),
        Case('main', base + ['--length', '8', '--count', '3', '--first-char-not-lc'] + s('charset', 'äabc')),
    ]
    for frac in ['>=60', '*>=70', '<20']:
        out.append(Case('main', base + ['--words', '4', '--length', '5', '--combine', '--count', '3'] +
                        s('passphr-len', frac)))
    return out


def cases_lua(rng):
    out = []
    ex = os.path.join(ROOT, 'upstream', 'script_examples')
    for sc in SCRIPTS:
        for extra in [['--length', '12'], ['--words', '4'], ['--length', '8', '--words', '3', '--combine'],
                      ['--format', '4d-P'], ['--length', '10', '--words', '2', '--count', '4', '--check-each']]:
            args = ['lua', '--script', os.path.join(ex, sc), '--seed-passw', 'lua-' + sc] + extra
            if '--count' not in extra:
                args += ['--count', '3']
            out.append(Case('lua', args))
    for sc in ['lua_standalone.lua', 'lua_modify.lua', 'lua_empty.lua', 'lua_unicode.lua', 'lua_bigsec.lua',
               'lua_error.lua', 'lua_nogen.lua']:
        for extra in [['--length', '10'], ['--words', '2', '--format', '"["P"]"'], []]:
            out.append(Case('lua', ['lua', '--script', fpath(sc), '--seed-passw', 'l', '--count', '3',
                                    '--dest', '1', '--advanced-flags', '0'] + extra))
    return out


EST_FIXED = ['', 'a', 'aa', 'aaaa', 'abc', 'abcd', 'password', 'Tr0ub4dor&3', 'correcthorsebatterystaple',
             'P@ssw0rd!', '\x01\x02\x1f', ' ', '~~~~', 'zyxwvu', '13579', 'AbCdEfGh', 'äöü', 'жжж', '€uro',
             'aä', 'a😀', '😀', '😀😀', '😀a😀', '漢字テスト', '\x7f\x80\x9f', 'x' * 100]


def cases_estimate(rng):
    out = []
    alph = [''.join(chr(c) for c in range(32, 127)), 'abcdef', '0123456789', ''.join(chr(c) for c in range(1, 32)),
            'äöüßÄÖÜéèê', 'абвгдежзий', '中文字漢', '!@#$%^&*()']
    for t in EST_FIXED:
        out.append(Case('estimate' if not any(ord(c) > 0xffff for c in t) else 'estimate-nonbmp',
                        ['estimate'] + s('passw', t)))
    for _ in range(170):
        a = ''.join(rng.sample(alph, rng.randint(1, 3)))
        t = ''.join(rng.choice(a) for _ in range(rng.randint(1, 40)))
        out.append(Case('estimate', ['estimate'] + s('passw', t)))
    for _ in range(10):
        t = ''.join(rng.choice(['😀', '🎉', 'a', 'b', '1']) for _ in range(rng.randint(1, 10)))
        if any(ord(c) > 0xffff for c in t):
            out.append(Case('estimate-nonbmp', ['estimate'] + s('passw', t)))
    return out


ZX_FIXED = ['', 'password', 'correcthorsebatterystaple', 'Tr0ub4dor&3', 'qwerty', 'qwertyuiop', 'zxcvbnm', '123456',
            '1234567890', 'abcdef', 'aaaaaaa', 'abababab', '19841224', '12/24/1984', '2015-06-01', 'p@55w0rd',
            'P4$$w0rD!', 'iloveyou', 'monkey123', 'dragon', 'trustno1', 'asdfghjkl;', '1qaz2wsx', 'Zxcvbn-c',
            'äöü', 'пароль', '😀😀😀', 'naïve', 'x' * 60, 'The quick brown fox jumps over the lazy dog',
            'rosebud1984', 'J0hnSm1th', '!@#$%^&*', '9876543210', 'aB3$eF6&hI9(', 'mississippi', 'abc123xyz789',
            'Password1', 'letmein!', 'baseball', 'hunter2', 'q1w2e3r4t5', '1q2w3e4r5t', 'zaq12wsx', 'P@ssword']


def cases_zxcvbn(rng):
    out = []
    for t in ZX_FIXED:
        out.append(Case('zxcvbn', ['zxcvbn'] + s('passw', t)))
    alph = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()-_=+'
    words = ['love', 'dragon', 'master', 'sun', 'shine', 'pass', 'word', 'admin', 'qwerty', 'football', 'star']
    for _ in range(80):
        c = rng.random()
        if c < 0.4:
            t = ''.join(rng.choice(alph) for _ in range(rng.randint(1, 24)))
        elif c < 0.8:
            t = ''.join(rng.choice(words) + rng.choice(['', '1', '!', '2020', '_']) for _ in range(rng.randint(1, 4)))
        else:
            t = ''.join(rng.choice('äöüжё€😀abc123') for _ in range(rng.randint(1, 12)))
        out.append(Case('zxcvbn', ['zxcvbn'] + s('passw', t)))
    return out


def cases_mppg(rng):
    out = []
    for pw in ['master', 'пароль', 'a😀', ' ', 'x' * 50]:
        for param in ['example.com', '', 'äöü', 'site😀']:
            for ci in range(6):
                if rng.random() < 0.5:
                    continue
                ln = rng.choice([0, 1, 8, 16, 100])
                args = ['mppg'] + s('passw', pw) + (s('param', param) if param else []) + \
                    ['--charset-index', str(ci), '--length', str(ln)]
                if rng.random() < 0.4:
                    args.append('--add-length')
                out.append(Case('mppg', args))
            out.append(Case('mppg', ['mppg'] + s('passw', pw) + (s('param', param) if param else []) + ['--hashapass']))
    for ln in [255, 256, 257, 300, 1000, 65536 + 5]:
        out.append(Case('mppg', ['mppg', '--passw', 'm', '--param', 'p', '--length', str(ln), '--add-length']))
    return out


# --- text encryption: old-version vectors are built here independently
# (AES via the openssl command line tool, LZO1X literal-only stream)

def lzo_literal_stream(data):
    n = len(data)
    if n == 0:
        return bytes([0x11, 0, 0])
    if n <= 238:
        return bytes([17 + n]) + data + bytes([0x11, 0, 0])
    # long literal run: t = 0, then 0-bytes (255 each), then remainder
    t = n - 3 - 15
    out = bytearray([0])
    while t > 255:
        out.append(0)
        t -= 255
    out.append(t)
    return bytes(out) + data + bytes([0x11, 0, 0])


def aes_cbc_encrypt(key, iv, data):
    p = subprocess.run(['openssl', 'enc', '-aes-256-cbc', '-K', key.hex(), '-iv', iv.hex(), '-nopad'],
                       input=data, capture_output=True, check=True)
    return p.stdout


def b64_lines(data):
    t = base64.b64encode(data).decode()
    return '\r\n'.join(t[i:i + 76] for i in range(0, len(t), 76))


def crypt_old_vector(text_bytes, passw_bytes, version, rnd):
    iv = bytearray(rnd.randbytes(16))
    if version == 0:
        key = bytearray(32)
        key[:16] = iv
        key = bytes(key)
        for _ in range(8192):
            key = hashlib.sha256(key + passw_bytes).digest()
        header = len(text_bytes).to_bytes(4, 'little')
    else:
        key = hashlib.pbkdf2_hmac('sha256', passw_bytes, bytes(iv), 8192, 32)
        header = b'PWG' + bytes([version]) + len(text_bytes).to_bytes(4, 'little')
    data = header + lzo_literal_stream(text_bytes)
    last = len(data) % 16
    iv[15] = (iv[15] & 0xf0) | last
    if version == 0:
        # key derived from the final IV (salt)
        key = bytearray(32)
        key[:16] = iv
        key = bytes(key)
        for _ in range(8192):
            key = hashlib.sha256(key + passw_bytes).digest()
    else:
        key = hashlib.pbkdf2_hmac('sha256', passw_bytes, bytes(iv), 8192, 32)
    if last:
        data += rnd.randbytes(16 - last)
    enc = aes_cbc_encrypt(key, bytes(iv), data)
    if version == 0:
        mac = hmac.new(key, bytes(iv) + enc, hashlib.sha256).digest()[:16]
    else:
        mac = hmac.new(key, data, hashlib.sha256).digest()
    return b64_lines(bytes(iv) + enc + mac)


TEXTS = ['Hello', 'x', 'Hello, world! ' * 20, 'Line1\r\nLine2\r\n', 'Пароль: секрет', 'äöü ß € 😀',
         'a' * 1000, ''.join(chr(0x4e00 + i) for i in range(300)), '\t tab \t', '0123456789abcdef' * 4]
CT_PASSWS = ['secret', '', 'пароль', 'ä😀', 'x' * 100, 'pass word']


def cases_crypt(rng):
    enc = []
    for t in TEXTS + ['']:
        for pw in rng.sample(CT_PASSWS, 3):
            seed = rng.randbytes(rng.randint(1, 16))
            enc.append(Case('crypt-enc', ['crypt-text-enc'] + s('passw', pw) + s('text', t) + b('rand-seed', seed)))
    dec = []
    rnd = random.Random(SEED + 7)
    vec_specs = []
    for version in [0, 1, 2]:
        for t, pw in [('Hello old', 'secret'), ('Привет', 'пароль'), ('äöü', 'pw'), ('x' * 300, 'long'),
                      ('Grüße', 'ä')]:
            for acp in [1251, 1252]:
                if version < 2:
                    enc_name = 'cp%d' % acp
                    tb = t.encode(enc_name, 'replace')
                    pb = pw.encode(enc_name, 'replace')
                else:
                    tb = t.encode('utf-8')
                    pb = pw.encode('utf-16-le')
                vec_specs.append((version, t, pw, acp, crypt_old_vector(tb, pb, version, rnd)))
    for version, t, pw, acp, vec in vec_specs:
        dec.append(Case('crypt-dec-old', ['crypt-text-dec'] + s('passw', pw) + s('text', vec) +
                        ['--version', str(version)], acp=acp, note='v%d vector' % version))
        dec.append(Case('crypt-dec-old', ['crypt-text-dec', '--old'] + s('passw', pw) + s('text', vec), acp=acp,
                        note='v%d vector, --old' % version))
        dec.append(Case('crypt-dec-old', ['crypt-text-dec', '--old'] + s('passw', pw + 'x') + s('text', vec),
                        acp=acp, note='v%d vector, wrong password' % version))
    for bad in ['abc', '!!!!', 'AAAA' * 16, 'AAAA' * 17, base64.b64encode(bytes(64)).decode(),
                base64.b64encode(bytes(80)).decode(), ' ' * 10, 'QUJD\r\nREVG']:
        dec.append(Case('crypt-dec', ['crypt-text-dec', '--passw', 'x'] + s('text', bad)))
        dec.append(Case('crypt-dec', ['crypt-text-dec', '--old', '--passw', 'x'] + s('text', bad)))
    return enc, dec


def cases_crypt_cross(enc_cases):
    out = []
    for c in enc_cases:
        for side, res in (('ref', c.ref), ('port', c.port)):
            if not res or not res.get('ok') or res.get('result') != 0:
                continue
            text = res['text']
            pw = None
            a = c.args
            for i, x in enumerate(a):
                if x == '--passw':
                    pw = a[i + 1]
                elif x == '--passw-u16hex':
                    pw = bytes.fromhex(a[i + 1]).decode('utf-16-le')
            out.append(Case('crypt-dec', ['crypt-text-dec'] + s('passw', pw) + s('text', text),
                            note='ciphertext from %s harness (case %d)' % (side, c.id)))
            if side == 'ref':
                out.append(Case('crypt-dec', ['crypt-text-dec', '--old'] + s('passw', pw) + s('text', text),
                                note='--old, ciphertext from ref (case %d)' % c.id))
                out.append(Case('crypt-dec', ['crypt-text-dec', '--version', '1'] + s('passw', pw) + s('text', text),
                                note='as version 1, ciphertext from ref (case %d)' % c.id))
                mangled = text.replace('A', 'B', 1) if 'A' in text else text[:-4]
                out.append(Case('crypt-dec', ['crypt-text-dec'] + s('passw', pw) + s('text', mangled),
                                note='modified ciphertext (case %d)' % c.id))
    return out


def cases_keyfile(rng):
    out = []
    for f in ['key32.bin', 'key64hex_upper.txt', 'key64hex_lower.txt', 'key64hex_mixed.txt', 'key64_nonhex.txt',
              'key63.bin', 'key65.bin', 'key1.bin', 'key_empty.bin', 'key64hex_nl.txt', 'key_big.bin',
              'key_text.txt', 'missing.key']:
        out.append(Case('keyfile', ['keyfile', '--in', fpath(f)]))
    for sd in ['01', 'abcdef', '00' * 32]:
        out.append(Case('keyfile', ['keyfile-create', '--out', '{OUT}'], glob=['--det-rand-seed', sd]))
    return out


def cases_trigram(rng):
    out = []
    for src, glob, acp in [('tri_src_utf8.txt', [], 1251), ('tri_src_utf16.txt', [], 1251),
                           ('tri_src_small.txt', [], 1251), ('tri_src_none.txt', [], 1251),
                           ('wl_big.txt', [], 1251), ('wl_ansi1251.txt', ['--file-encoding', '0'], 1251),
                           ('wl_utf8.txt', ['--file-encoding', '0'], 1252), ('wl_long.txt', [], 1251),
                           ('wl_badutf8.txt', [], 1251), ('missing.txt', [], 1251)]:
        out.append(Case('trigram', ['trigram-create', '--src', fpath(src), '--dst', '{OUT}'], glob=glob, acp=acp))
    return out


def trigram_use_cases(tri_cases):
    out = []
    for c in tri_cases:
        if c.ref and c.ref.get('ok') and c.port and c.port.get('ok'):
            for fl in [0, 0x100, 0x80, 3]:
                out.append(Case('phonetic', ['gen', '--seed-passw', 'tri', '--trigram', c.out_file('ref'), '--mode',
                                             'phonetic', '--length', '16', '--flags', str(fl), '--count', '3'],
                                port_args=['gen', '--seed-passw', 'tri', '--trigram', c.out_file('port'), '--mode',
                                           'phonetic', '--length', '16', '--flags', str(fl), '--count', '3'],
                                note='trigram file of case %d (each side its own file)' % c.id))
    return out


# --- databases

def db_specs(rng):
    def entry(i, **kw):
        e = {'title': 'Entry %d' % i, 'user_name': 'user%d' % i, 'password': 'pw%d' % i, 'url': 'https://e%d.org' % i,
             'keyword': 'kw', 'notes': 'line1\r\nline2', 'creation_time': 133000000000000000 + i,
             'modification_time': 133000000100000000 + i}
        e.update(kw)
        return e

    full = entry(1, key_values=[['k', 'v'], ['Schlüssel', 'Wert=1'], ['a=b', 'c'], ['empty', '']],
                 tags=['b', 'a', 'ä', 'ab', 'Ж', '😀', 'aa', 'A', 'z', 'B'],
                 passw_change_time=133000000200000000, passw_expiry_date='2026-12-31',
                 history={'active': True, 'max_size': 3,
                          'entries': [{'time': 132000000000000000, 'password': 'old1'},
                                      {'time': 131000000000000000, 'password': 'old2'},
                                      {'time': 0, 'password': 'old3'}, {'time': 1, 'password': 'old4'}]})
    specs = {
        'minimal': {'password': 'master', 'kdf_iterations': 1000, 'rand_seed_hex': '01'},
        'aes_full': {'password': 'master', 'kdf_iterations': 1000, 'rand_seed_hex': '0102', 'compressed': True,
                     'compression_level': 6, 'default_user_name': 'def', 'passw_format_seq': '{8x}',
                     'description': 'Test DB äö😀', 'default_passw_expiry_days': 90,
                     'default_max_passw_history_size': 5, 'master_passw_expiry_date': {'y': 2027, 'm': 1, 'd': 31},
                     'entries': [full, entry(2), entry(3, user_name=None), {'title': 'empty'}]},
        'chacha_full': {'password': 'master', 'cipher': 1, 'kdf_iterations': 1000, 'rand_seed_hex': '03',
                        'entries': [full, entry(2)]},
        'chacha_compressed': {'password': 'master', 'cipher': 1, 'kdf_iterations': 1000, 'rand_seed_hex': '04',
                              'compressed': True, 'compression_level': 9, 'entries': [entry(i) for i in range(20)]},
        'recovery': {'password': 'master', 'recovery_password': 'rescue', 'kdf_iterations': 1000,
                     'rand_seed_hex': '05', 'entries': [full]},
        'recovery_chacha': {'password': 'master', 'recovery_password': 'rescue', 'cipher': 1, 'kdf_iterations': 500,
                            'rand_seed_hex': '06', 'entries': [entry(1)]},
        'keyfile': {'password': 'master', 'keyfile': fpath('key32.bin'), 'kdf_iterations': 1000,
                    'rand_seed_hex': '07', 'entries': [entry(1)]},
        'keyfile_only': {'password': '', 'keyfile': fpath('key64hex_upper.txt'), 'kdf_iterations': 1000,
                         'rand_seed_hex': '08', 'entries': [entry(1)]},
        'recovery_keyfile': {'password': 'm', 'recovery_keyfile': fpath('key_text.txt'), 'kdf_iterations': 1000,
                             'rand_seed_hex': '09', 'entries': [entry(1)]},
        'utf16': {'password': 'пароль😀', 'passw_utf8': False, 'kdf_iterations': 1000, 'rand_seed_hex': '0a',
                  'entries': [entry(1)]},
        'utf8_nonascii': {'password': 'пароль😀', 'kdf_iterations': 1000, 'rand_seed_hex': '0b',
                          'entries': [entry(1, password='пароль😀ä', title='Заголовок', notes='😀\r\n' * 50)]},
        'expiry': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '0c',
                   'master_passw_expiry_date': {'y': 2026, 'm': 13, 'd': 1},
                   'entries': [entry(1, passw_expiry_date={'y': 2026, 'm': 2, 'd': 30}),
                               entry(2, passw_expiry_date=0), entry(3, passw_expiry_date=(2030 << 16) | (6 << 8) | 15),
                               entry(4, passw_expiry_date='1600-01-01'), entry(5, passw_expiry_date='9999-12-31'),
                               entry(6, passw_expiry_date={'y': 0, 'm': 0, 'd': 0})]},
        'history': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '0d', 'default_max_passw_history_size': 2,
                    'entries': [entry(1, history={'active': False, 'max_size': 1,
                                                  'entries': [{'time': 5, 'password': 'h'}]}),
                                entry(2, history={'active': True, 'max_size': 255,
                                                  'entries': [{'time': i, 'password': 'h%d' % i} for i in range(300)]}),
                                entry(3, history={'active': True, 'max_size': 0, 'entries': []}),
                                entry(4)]},
        'many': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '0e', 'compressed': False,
                 'entries': [entry(i, password='p' * (i % 17)) for i in range(100)]},
        'big_compressed': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '0f', 'compressed': True,
                           'compression_level': 1,
                           'entries': [entry(1, notes='Lorem ipsum dolor sit amet ' * 2000)]},
        'level0': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '10', 'compressed': True,
                   'compression_level': 0, 'entries': [entry(1)]},
        'default_kdf': {'password': 'master', 'rand_seed_hex': '11', 'entries': [entry(1)]},
        'tags_order': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '12',
                       'entries': [entry(1, tags=['ё', 'е', 'Z', 'a', 'Ä', '😀', '😁', '￿', 'ab', 'b', 'AB',
                                                   'abc', '1', ' '])]},
        'kv_edge': {'password': 'x', 'kdf_iterations': 1000, 'rand_seed_hex': '13',
                    'entries': [entry(1, key_values=[['=', '='], ['k', 'v\nw'], ['', 'x'], ['k2', '=='], ['ä', '😀']])]},
    }
    return specs


def cases_db_create(rng):
    out = []
    specs = db_specs(rng)
    for name, spec in specs.items():
        path = write('db_%s.json' % name, json.dumps(spec, ensure_ascii=False).encode('utf-8'))
        c = Case('db-create', ['db-create', '--spec', path, '--out', '{OUT}'], timeout=300, note=name)
        c.spec = spec
        c.spec_name = name
        out.append(c)
    # random salts (no rand_seed_hex): only cross-reading is meaningful
    spec = dict(specs['aes_full'])
    del spec['rand_seed_hex']
    path = write('db_random.json', json.dumps(spec, ensure_ascii=False).encode('utf-8'))
    c = Case('db-create-random', ['db-create', '--spec', path, '--out', '{OUT}'], timeout=300, note='random salts',
             ignore={'file_hex', 'file_sha256'})
    c.spec = spec
    c.spec_name = 'random'
    out.append(c)
    return out


def dump_args(spec, use_recovery=False):
    utf8 = spec.get('passw_utf8', True)
    args = ['db-dump']
    if use_recovery:
        args += s('passw', spec.get('recovery_password') or '')
        if spec.get('recovery_keyfile'):
            args += ['--keyfile', spec['recovery_keyfile']]
    else:
        args += s('passw', spec.get('password', ''))
        if spec.get('keyfile'):
            args += ['--keyfile', spec['keyfile']]
    return args, utf8


def cases_db_dump(create_cases):
    out = []
    cross = []
    for c in create_cases:
        spec = c.spec
        files = {}
        for side, res in (('ref', c.ref), ('port', c.port)):
            if res and res.get('ok') and os.path.exists(c.out_file(side)):
                files[side] = c.out_file(side)
        for side, f in files.items():
            args, _ = dump_args(spec)
            out.append(Case('db-dump', args + ['--in', f], timeout=300,
                            note='%s: file created by %s harness' % (c.spec_name, side)))
            if spec.get('recovery_password') or spec.get('recovery_keyfile'):
                args, _ = dump_args(spec, True)
                out.append(Case('db-dump', args + ['--in', f], timeout=300,
                                note='%s: recovery key, file by %s' % (c.spec_name, side)))
            out.append(Case('db-dump', ['db-dump', '--header-only', '--in', f],
                            note='%s: header, file by %s' % (c.spec_name, side)))
            args, _ = dump_args(spec)
            wrong = list(args)
            for i, x in enumerate(wrong):
                if x.startswith('--passw'):
                    wrong[i:i + 2] = s('passw', 'wrong')
                    break
            out.append(Case('db-dump', wrong + ['--in', f], timeout=300,
                            note='%s: wrong password, file by %s' % (c.spec_name, side)))
            if spec.get('passw_utf8', True) and spec.get('password'):
                out.append(Case('db-dump', args + ['--utf16-passw', '--in', f], timeout=300,
                                note='%s: --utf16-passw, file by %s' % (c.spec_name, side)))
        if c.spec_name in ('aes_full', 'chacha_full') and 'ref' in files:
            data = open(files['ref'], 'rb').read()
            for kind, mod in [('flipped byte', data[:200] + bytes([data[200] ^ 1]) + data[201:]),
                              ('truncated', data[:len(data) - 7]), ('header only', data[:18]),
                              ('bad magic', b'XXXX' + data[4:]),
                              ('bad version', data[:5] + b'\x09\x01' + data[7:])]:
                p = write('dbmod_%s_%s.pwdb' % (c.spec_name, kind.replace(' ', '_')), mod)
                args, _ = dump_args(spec)
                out.append(Case('db-dump', args + ['--in', p], timeout=300, note='%s: %s' % (c.spec_name, kind)))
        if len(files) == 2:
            cross.append((c, files))
    return out, cross


# ---------------------------------------------------------------------------
# report

def fmt_val(v, n=300):
    t = json.dumps(v, ensure_ascii=False) if not isinstance(v, str) else repr(v)
    return t if len(t) <= n else t[:n] + '…(%d chars)' % len(t)


def sanitize(text):
    """Remove local details from the report: absolute paths (also as seen by
    wine), the home directory and user name, and values that contain
    uninitialized memory of the reference harness (control characters)."""
    home = os.path.expanduser('~')
    user = os.path.basename(home)
    win_root = 'Z:' + ROOT.replace('/', '\\\\')
    text = text.replace(win_root, 'Z:\\\\<repo>').replace(win_root.replace('\\\\', '\\'), 'Z:\\<repo>')
    text = text.replace(ROOT + '/', '').replace(ROOT, '.')
    text = text.replace(home, '~')
    if user:
        text = re.sub(r'\b%s\b' % re.escape(user), '<user>', text)

    def mask(m):
        v = m.group(0)
        if re.search(r'\\x[01][0-9a-f]', v):
            return "<uninitialized memory, %d characters>" % len(v)
        return v
    return re.sub(r"'[^'\n]{8,}'", mask, text)


def write_report(path, areas, cases, cross_results, extra_notes, runtime):
    lines = []
    lines.append('# Сверка Rust-порта с оригинальным ядром Password Tech 3.6.1')
    lines.append('')
    lines.append('Сгенерировано `tools/refcompare/compare.py` (%s, %.0f с). Эталон — `tools/refharness` '
                 '(неизменённые исходники оригинала под wine), порт — `crates/pwtech-core/examples/portharness.rs`. '
                 'Входные данные детерминированы (seed %d).' % (time.strftime('%Y-%m-%d %H:%M'), runtime, SEED))
    lines.append('')
    lines.append('Повторить: `python3 -I tools/refcompare/compare.py` (опции: `-j N`, `--area a,b`, `--no-build`).')
    lines.append('')
    lines.append('Статусы: **match** — все общие поля совпали; **both-error** — обе стороны вернули ошибку '
                 '(классы исключений совпали или несравнимы); **skipped** — вход не представим в API порта; '
                 '**mismatch** — расхождение.')
    lines.append('')
    lines.append('| область | случаев | match | both-error | skipped | mismatch | сравнено полей |')
    lines.append('|---|---:|---:|---:|---:|---:|---:|')
    tot = [0, 0, 0, 0, 0, 0]
    for a in areas:
        cs = [c for c in cases if c.area == a]
        if not cs:
            continue
        cnt = [len(cs), sum(c.status == 'match' for c in cs), sum(c.status in ('both-error', 'both-timeout') for c in cs),
               sum(c.status == 'skipped' for c in cs), sum(c.status == 'mismatch' for c in cs),
               sum(c.compared for c in cs)]
        tot = [x + y for x, y in zip(tot, cnt)]
        lines.append('| %s | %d | %d | %d | %d | %d | %d |' % (a, *cnt))
    lines.append('| **всего** | %d | %d | %d | %d | %d | %d |' % tuple(tot))
    lines.append('')
    if cross_results:
        lines.append('Перекрёстное чтение баз: %d проверок (дамп оригиналом файла порта = дамп оригиналом своего '
                     'файла), расхождений: %d.' % (len(cross_results), sum(1 for r in cross_results if not r[1])))
        lines.append('')
    for n in extra_notes:
        lines.append(n)
    mism = [c for c in cases if c.status == 'mismatch']
    lines.append('## Расхождения (%d)' % len(mism))
    lines.append('')
    if not mism:
        lines.append('Нет.')
        lines.append('')
    for c in mism:
        lines.append('### #%d %s%s' % (c.id, c.area, (' — ' + c.note) if c.note else ''))
        r, p = c.repro()
        lines.append('```')
        lines.append(r)
        lines.append(p)
        lines.append('```')
        for path_, rv, pv in c.diffs[:6]:
            lines.append('- `%s`: эталон %s, порт %s' % (path_ or '/', fmt_val(rv), fmt_val(pv)))
        if len(c.diffs) > 6:
            lines.append('- … ещё %d' % (len(c.diffs) - 6))
        lines.append('')
    for name, ok, detail in cross_results:
        if not ok:
            lines.append('- перекрёстное чтение `%s`: %s' % (name, detail))
    both = [c for c in cases if c.status in ('both-error', 'both-timeout')]
    lines.append('## Случаи, где обе стороны вернули ошибку (%d)' % len(both))
    lines.append('')
    seen = set()
    for c in both:
        key = (c.area, c.ref.get('exception'), c.port.get('exception'))
        if key in seen:
            continue
        seen.add(key)
        lines.append('- %s #%d: эталон `%s: %s`, порт `%s: %s`' % (
            c.area, c.id, c.ref.get('exception'), str(c.ref.get('error'))[:120], c.port.get('exception'),
            str(c.port.get('error'))[:120]))
    lines.append('')
    sk = [c for c in cases if c.status == 'skipped']
    if sk:
        lines.append('## Пропущенные (%d)' % len(sk))
        lines.append('')
        for c in sk[:20]:
            lines.append('- %s #%d: %s' % (c.area, c.id, c.note))
        lines.append('')
    with open(path, 'w', encoding='utf-8') as f:
        f.write(sanitize('\n'.join(lines) + '\n'))


# ---------------------------------------------------------------------------

AREAS = ['prng', 'hash', 'chacha', 'charsets', 'password', 'phonetic', 'passphrase', 'format', 'main', 'lua',
         'estimate', 'estimate-nonbmp', 'zxcvbn', 'mppg', 'crypt-enc', 'crypt-dec', 'crypt-dec-old', 'keyfile',
         'trigram', 'db-create', 'db-create-random', 'db-dump']


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('-j', '--jobs', type=int, default=max(2, (os.cpu_count() or 4)))
    ap.add_argument('--area', default='', help='comma-separated list of areas (default: all)')
    ap.add_argument('--no-build', action='store_true')
    ap.add_argument('--report', default=DEFAULT_REPORT)
    ap.add_argument('--keep', action='store_true', help='keep old work files')
    ap.add_argument('--dump', default='', help='write all results as JSON lines to this file')
    args = ap.parse_args()
    sel = set(a for a in args.area.split(',') if a)

    t0 = time.time()
    if not args.no_build:
        print('building portharness ...', file=sys.stderr)
        subprocess.run(['cargo', 'build', '-q', '--release', '-p', 'pwtech-core', '--example', 'portharness'],
                       cwd=ROOT, check=True)
    if not args.keep and os.path.isdir(WORK):
        shutil.rmtree(WORK)
    make_files()

    rng = random.Random(SEED)
    gens = [('prng', cases_prng), ('hash', cases_hash), ('chacha', cases_chacha), ('charsets', cases_charsets),
            ('password', cases_password), ('phonetic', cases_phonetic), ('passphrase', cases_passphrase),
            ('format', cases_format), ('main', cases_main), ('lua', cases_lua), ('estimate', cases_estimate),
            ('zxcvbn', cases_zxcvbn), ('mppg', cases_mppg), ('keyfile', cases_keyfile), ('trigram', cases_trigram)]
    phase1 = []
    for name, fn in gens:
        cs = fn(random.Random('%d-%s' % (SEED, name)))
        phase1 += cs
    enc, dec = cases_crypt(random.Random('%d-crypt' % SEED))
    phase1 += enc + dec
    dbc = cases_db_create(rng)
    phase1 += dbc

    def selected(c):
        return not sel or c.area in sel or c.area.split('-')[0] in sel

    phase1 = [c for c in phase1 if selected(c)]
    print('phase 1: %d cases' % len(phase1), file=sys.stderr)
    run_all(phase1, args.jobs, 'phase 1')

    phase2 = []
    phase2 += [c for c in cases_crypt_cross([c for c in enc if c.ref]) if selected(c)]
    phase2 += [c for c in trigram_use_cases([c for c in phase1 if c.area == 'trigram']) if selected(c)]
    dumps, cross = cases_db_dump([c for c in dbc if c.ref])
    phase2 += [c for c in dumps if selected(c)]
    print('phase 2: %d cases' % len(phase2), file=sys.stderr)
    run_all(phase2, args.jobs, 'phase 2')
    allc = phase1 + phase2

    # cross-reading: the original reads the port's file exactly like its own
    cross_results = []
    by_note = {}
    for c in dumps:
        by_note[c.note] = c
    for c, files in cross:
        a = by_note.get('%s: file created by ref harness' % c.spec_name)
        bb = by_note.get('%s: file created by port harness' % c.spec_name)
        if not a or not bb or not a.ref or not bb.ref:
            continue
        ra, rb = dict(a.ref), dict(bb.ref)
        for x in (ra, rb):
            for k in ('file_size', 'file_sha256', 'layout', 'acp'):
                x.pop(k, None)
            for e in x.get('entries', []):
                e.pop('strings_locale', None)
        same = json.dumps(ra, sort_keys=True) == json.dumps(rb, sort_keys=True)
        cross_results.append((c.spec_name, same, '' if same else 'dump of port file differs from dump of ref file'))

    runtime = time.time() - t0
    notes = []
    write_report(args.report, AREAS, allc, cross_results, notes, runtime)
    if args.dump:
        with open(args.dump, 'w', encoding='utf-8') as f:
            for c in allc:
                f.write(json.dumps({'id': c.id, 'area': c.area, 'status': c.status, 'note': c.note,
                                    'args': c.args, 'glob': c.glob, 'acp': c.acp, 'ref': c.ref, 'port': c.port,
                                    'diffs': [list(map(str, d)) for d in c.diffs]}, ensure_ascii=False) + '\n')
    nm = sum(c.status == 'mismatch' for c in allc)
    print('%d cases, %d mismatches, %.0fs; report: %s' % (len(allc), nm, runtime, args.report), file=sys.stderr)
    return 1 if nm else 0


if __name__ == '__main__':
    sys.exit(main())
