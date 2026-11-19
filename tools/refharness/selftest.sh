#!/usr/bin/env bash
# Smoke test of the reference harness: runs a few commands of each kind and
# checks the results (known test vectors where they exist, round trips
# otherwise). Usage: selftest.sh   (build first with build.sh)
set -uo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RUN="$HERE/run.sh"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/refharness-selftest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

PASS=0
FAIL=0

# check NAME EXPR CMD... : run the harness, evaluate the Python expression
# EXPR on the parsed JSON object `d`; success if it is truthy
check() {
  local name="$1" expr="$2"
  shift 2
  local out
  out=$("$RUN" "$@" 2>>"$TMP/stderr.log")
  if printf '%s' "$out" | python3 -I -c '
import json, sys
d = json.loads(sys.stdin.read())
ok = d.get("ok") and bool(eval(sys.argv[1]))
sys.exit(0 if ok else 1)' "$expr"; then
    PASS=$((PASS + 1))
    echo "ok   $name"
  else
    FAIL=$((FAIL + 1))
    echo "FAIL $name"
    echo "     $out" | cut -c1-400
  fi
}

# get FIELD CMD... : print one top-level field of the JSON result
get() {
  local field="$1"
  shift
  "$RUN" "$@" 2>>"$TMP/stderr.log" | python3 -I -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$field"
}

Z64=0000000000000000000000000000000000000000000000000000000000000000

# --- primitives (standard test vectors) ---
check "sha256 abc" 'd["hash"]=="ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"' \
  sha256 --data-utf8 abc
check "hmac-sha256 rfc4231#1" 'd["mac"]=="b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"' \
  hmac-sha256 --key-hex 0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b --data-utf8 "Hi There"
check "hmac-sha512 rfc4231#1" 'd["mac"].startswith("87aa7cdea5ef619d4ff0b4241a1d6cb0")' \
  hmac-sha512 --key-hex 0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b --data-utf8 "Hi There"
check "pbkdf2 rfc7914" 'd["key"]=="120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"' \
  pbkdf2 --passw-utf8 password --salt-utf8 salt --iter 1
check "chacha20 zero key" 'd["keystream"].startswith("76b8e0ada0f13d90405d6ae55386bd28")' \
  chacha --key-hex $Z64 --iv-hex 0000000000000000 --bytes 64
check "chacha8 zero key" 'd["keystream"].startswith("3e00ef2f895f40d67f5bb8e81f09a5a1")' \
  chacha --key-hex $Z64 --iv-hex 0000000000000000 --rounds 8 --bytes 16

# --- AESCtrPRNG ---
# Seed(00): key = SHA-256(00), output = AES-256-ECB(key, counter 0, 1, ...)
check "aesctr seed" 'd["data"]=="b6e601deddb2cf246ad71d9a4b292b4491b496da0742a4ae981e210d489c43aa"' \
  prng-aesctr --seed-hex 00 --bytes 32
check "aesctr rekey after 1 MiB" 'd["data"].startswith("2b17e8b72cca4e0c")' \
  prng-aesctr --seed-hex 00 --skip 1048512 --bytes 128
check "aesctr numrange <=256" 'len(d["values"])==20 and all(v<7 for v in d["values"])' \
  prng-aesctr --key-hex 0001 --numrange 7 --count 20
check "aesctr numrange <=65536" 'len(d["values"])==20 and all(v<1000 for v in d["values"])' \
  prng-aesctr --key-hex 0001 --param-hex 0203 --numrange 1000 --count 20
check "aesctr numrange >65536" 'len(d["values"])==20 and all(v<100000 for v in d["values"])' \
  prng-aesctr --key-hex 0001 --numrange 100000 --count 20

# --- password generator ---
check "gen password" 'len(d["passwords"])==3 and all(len(p["password"])==16 for p in d["passwords"])' \
  gen --seed-passw test --charset "<AZ><az><09>" --length 16 --count 3
check "gen passphrase" 'len(d["passwords"][0]["password"].split(" "))==5' \
  gen --seed-passw test --mode passphrase --words 5 --flags 2
check "gen format" 'd["passwords"][0]["len"]==8 and d["passwords"][0]["security"]>0' \
  gen --seed-passw test --mode format --format "uuuudddd"
check "gen phonetic" 'd["passwords"][0]["password"].isalpha()' \
  gen --seed-passw test --mode phonetic --length 12
check "gen main" 'len(d["passwords"])==2' \
  gen --seed-passw test --mode main --length 8 --words 3 --combine --count 2
A=$(get passwords gen --seed-passw same --length 20 --count 2)
B=$(get passwords gen --seed-passw same --length 20 --count 2)
if [ "$A" = "$B" ]; then PASS=$((PASS + 1)); echo "ok   gen deterministic"; else FAIL=$((FAIL + 1)); echo "FAIL gen deterministic"; fi
check "estimate" 'd["security"]>0' estimate --passw "Tr0ub4dor&3"

# --- crypt text ---
ENC=$(get text crypt-text-enc --passw "secret" --text "Hello, world! Привет" --rand-seed-hex 01)
check "crypt-text round trip" 'd["text"]=="Hello, world! Привет"' \
  crypt-text-dec --passw secret --text "$ENC"
check "crypt-text bad key" 'd["result_name"]=="BADKEY"' \
  crypt-text-dec --passw wrong --text "$ENC"

# --- database ---
cat > "$TMP/spec.json" <<'EOF'
{
  "password": "master", "cipher": 0, "kdf_iterations": 1000,
  "compressed": true, "compression_level": 6, "rand_seed_hex": "0102",
  "default_user_name": "def", "description": "Test",
  "entries": [
    { "title": "E1", "user_name": "u", "password": "pässwörd", "notes": "n1\r\nn2",
      "key_values": [["k", "v"]], "tags": ["b", "a", "cc"],
      "creation_time": 133000000000000000, "modification_time": 133000000100000000,
      "passw_expiry_date": "2026-12-31",
      "history": {"active": true, "max_size": 3, "entries": [{"time": 132000000000000000, "password": "old"}]} }
  ]
}
EOF
check "db-create aes" 'd["file_size"]>0' db-create --spec "$TMP/spec.json" --out "$TMP/a.pwdb"
check "db-dump aes" 'd["db"]["description"]=="Test" and d["entries"][0]["password"]=="pässwörd" and d["entries"][0]["tags"]==["a","b","cc"] and d["entries"][0]["history"]["entries"][0]["password"]=="old"' \
  db-dump --in "$TMP/a.pwdb" --passw master
H1=$(get file_sha256 db-create --spec "$TMP/spec.json" --out "$TMP/b.pwdb")
H2=$(get file_sha256 db-create --spec "$TMP/spec.json" --out "$TMP/c.pwdb")
if [ "$H1" = "$H2" ]; then PASS=$((PASS + 1)); echo "ok   db-create deterministic"; else FAIL=$((FAIL + 1)); echo "FAIL db-create deterministic"; fi

printf '0123456789ABCDEF0123456789abcdef0123456789ABCDEF0123456789ABCDEF' > "$TMP/key.hex"
cat > "$TMP/spec2.json" <<'EOF'
{ "password": "pw", "keyfile": "key.hex", "recovery_password": "rec", "cipher": 1,
  "kdf_iterations": 500, "rand_seed_hex": "aa",
  "entries": [ { "title": "T", "password": "P" } ] }
EOF
check "db-create chacha+recovery+keyfile" 'd["file_size"]>0' db-create --spec "$TMP/spec2.json" --out "$TMP/r.pwdb"
check "db-dump master+keyfile" 'd["db"]["has_recovery_key"] and d["db"]["cipher_type"]==1 and d["entries"][0]["password"]=="P"' \
  db-dump --in "$TMP/r.pwdb" --passw pw --keyfile "$TMP/key.hex"
check "db-dump recovery" 'd["entries"][0]["password"]=="P"' db-dump --in "$TMP/r.pwdb" --passw rec
WRONG=$("$RUN" db-dump --in "$TMP/r.pwdb" --passw wrong 2>>"$TMP/stderr.log")
case "$WRONG" in
  *'"ok":false'*EPasswDbInvalidKey*) PASS=$((PASS + 1)); echo "ok   db-dump wrong password" ;;
  *) FAIL=$((FAIL + 1)); echo "FAIL db-dump wrong password"; echo "     $WRONG" ;;
esac

# --- key file ---
check "keyfile hex" 'd["key"]=="0123456789abcdef"*4' keyfile --in "$TMP/key.hex"
check "keyfile-create" 'len(d["content"])==64 and d["content"].lower()==d["key"]' \
  --det-rand-seed 01 keyfile-create --out "$TMP/new.key"

# --- trigram file ---
printf 'apple\nbanana\ncherry\ndate\nelderberry fig grape\nhoneydew kiwi lemon mango\n' > "$TMP/words.txt"
check "trigram-create" 'd["file_size"]==70316 and d["load_result"]==1' \
  trigram-create --src "$TMP/words.txt" --dst "$TMP/tri.bin"
check "gen wordlist" 'd["wordlist_load_result"]==11 and d["word_list_size"]==11' \
  gen --seed-passw x --mode passphrase --wordlist "$TMP/words.txt" --words 3

# --- MP password generator ---
check "mppg key" 'd["key"]=="48cab186e22c3282dcb2ae0ac73c25e08e34fa4bcbd549488d9175ebd93cc48f" and len(d["password"])==16' \
  mppg --passw master --param example.com --charset-index 0 --length 16
check "mppg hashapass" 'd["password"]=="ZdOyfVmH"' \
  mppg --passw master --param example.com --hashapass

# --- stretch ---
check "zxcvbn" 'd["entropy"]>0' zxcvbn --passw "correcthorsebatterystaple"
check "lua passphrase_mixedcase" 'len(d["passwords"])==2 and d["passwords"][0]["password"]!=""' lua --script "$HERE/../../upstream/script_examples/passphrase_mixedcase.lua" --seed-passw x --length 12 --words 3 --count 2
check "lua standalone" 'd["script_flags"]==1 and len(d["passwords"][0]["password"])==12' lua --script "$HERE/../../upstream/script_examples/alt_phonetic.lua" --seed-passw x --length 12

echo "passed: $PASS, failed: $FAIL"
[ "$FAIL" -eq 0 ]
