# refharness — эталонные результаты из оригинальных исходников Password Tech 3.6.1

Консольная Windows-программа, собранная из **неизменённых** исходников ядра Password Tech 3.6.1 (`upstream/src/...`) кросс-компилятором llvm-mingw и запускаемая под wine. Нужна, чтобы снимать с оригинального кода эталонные результаты (fixtures) для Rust-порта: вывод ГСЧ, пароли детерминированного генератора, зашифрованный текст, файлы баз, ключевые файлы, файлы триграмм, MP-генератор, zxcvbn, Lua-скрипты.

Каждый запуск печатает в stdout один JSON-объект (одна строка, UTF-8, двоичные данные — hex в нижнем регистре). Окон программа не открывает.

## Сборка

```
tools/refharness/build.sh          # инкрементально (make -j)
tools/refharness/build.sh clean    # с нуля
```

Нужны: `.ref/toolchain/llvm-mingw-20261117-ucrt-ubuntu-22.04-x86_64` (или `TOOLCHAIN_BIN=.../bin`), `make`, `python3`, подмодуль `upstream/`. Результат: `.ref/refharness/refharness.exe` (статически слинкован). Промежуточные файлы: `.ref/refharness/{gen,src,obj}`.

Проверка: `tools/refharness/selftest.sh` (≈35 проверок: тестовые векторы, детерминированность, round-trip базы).

## Запуск

```
tools/refharness/run.sh <команда> [параметры]
```

`run.sh` выставляет `WINEPREFIX=.ref/wineprefix`, `WINEDLLOVERRIDES="mscoree,mshtml="`, `WINEDEBUG=-all` и запускает exe. Мусор wine (dbus и т. п.) идёт в stderr, JSON — только в stdout.

- **Пути** можно давать линуксовые: программа сама превращает `/a/b` в `Z:\a\b`, `/` в `\`, относительные пути — в абсолютные (от текущего каталога). `winepath` не нужен.
- **Кодовая страница ANSI** (`CP_ACP`) берётся wine из локали (`LANG`/`LC_ALL`): `ru_RU.UTF-8` → 1251, `en_US.UTF-8` → 1252. От неё зависят набор `<highansi>`, чтение файлов в кодировке ANSI, `char*`→`UnicodeString`, старые версии зашифрованного текста. Текущее значение выводится в каждом ответе как `"acp"`. Пример: `LC_ALL=en_US.UTF-8 run.sh ...`.
- **Код возврата**: 0 — успех (`"ok":true`), 1 — ошибка (`{"ok":false,"exception":"<класс>","error":"<сообщение>"}`; класс — настоящий класс исключения оригинала, например `EPasswDbInvalidKey`), 2 — нет exe/неверный вызов.

### Общие параметры (до команды)

- `--det-rand-seed HEX` — `RandomPool::GetInstance()` начинает выдавать детерминированный поток (`AESCtrPRNG::Seed(HEX)`), `g_fastRandGen` получает фиксированное зерно. В ответ добавляется `"randpool_log"` — все запросы к пулу по порядку (hex). Так получаются воспроизводимые соли/IV/выравнивание базы и ключевые файлы.
- `--file-encoding N` — `g_config.FileEncoding` (0 ANSI, 1 UTF-16, 2 UTF-16BE, 3 UTF-8; по умолчанию 3, как в оригинале). Влияет на чтение словарей и источника триграмм без BOM.

### Как передавать данные

- Строковый параметр `NAME`: `--NAME текст` (из командной строки, UTF-16), `--NAME-u16hex HEX` (сырые UTF-16LE, можно с непарными суррогатами), `--NAME-file ФАЙЛ` (содержимое файла как UTF-8).
- Байтовый параметр `NAME`: `--NAME-hex HEX`, `--NAME-utf8 текст`, `--NAME-utf16 текст` (UTF-16LE).
- Можно `--opt=значение`.
- Числа с плавающей точкой выводятся как `%.17g` и дополнительно битами: поле `X_bits` = 16 hex-цифр IEEE-754 (big-endian), чтобы сравнивать бит в бит.

## Команды

### `prng-aesctr` — `AESCtrPRNG` (`random/AESCtrPRNG.cpp`)

```
run.sh prng-aesctr --key-hex 0001..1f --param-hex 0102 --bytes 16 --numrange 300 --count 4
{"ok":true,"cmd":"prng-aesctr","acp":1251,"seeding":"SeedWithKey","data":"ae798b62356105f47c3cbc15e2fc2f6f","numrange":300,"values":[112,29,20,9]}
```

Засев: `--key-hex K [--param-hex P]` → `SeedWithKey(K, P)` (пустой P → `nullptr, 0`), либо `--seed-hex S` → `Seed(S)`. Затем по порядку: `--skip N` (пропустить N байт), `--bytes N` → `data` (с `--digest` — только `data_sha256`), `--numrange M --count K` → `values` (`GetNumRange(M)`; ветки M≤256 / ≤65536 / больше), `--after-bytes N` → `after`.

### `pbkdf2`, `hmac-sha256`, `hmac-sha512`, `hmac-sha1`, `sha256`, `sha512`, `sha1`

```
run.sh pbkdf2 --passw-utf16 secret --salt-hex 00112233 --iter 8192     → "key"   (pbkdf2_256bit)
run.sh hmac-sha256 --key-utf8 k --data-utf8 d                          → "mac"   (hmac.h / polarssl)
run.sh sha256 --data-utf8 abc     (или --file ФАЙЛ)                    → "hash"
```

### `chacha` — `crypto/chacha.c` (вариант Бернштейна, nonce 64 бита, счётчик 64 бита)

```
run.sh chacha --key-hex <16|32 байта> --iv-hex <8 байт> [--ctr-hex <8 байт>] [--rounds 20] --bytes N
run.sh chacha ... --data-hex HEX            → "ciphertext" вместо "keystream"
run.sh chacha ... --bytes 80 --chunks 10    → обработка кусками (10 байт, затем остаток)
```

`--chunks a,b,...` показывает особенность `chacha_encrypt_bytes`: неполный 64-байтный блок отбрасывает остаток гаммы, следующий вызов начинает со следующего блока.

### `gen` — `PasswordGenerator` (`passw/PasswGen.cpp`)

Генератор получает `AESCtrPRNG`, засеянный как в оригинале:

- `--seed-passw ТЕКСТ` — Tools | Deterministic Random Generator (`Main.cpp:4215`): ключ = HMAC-SHA256(ключ = пароль в UTF-16LE, данные = `PWGen MP Password Generator`), `SeedWithKey(ключ, пусто)`;
- или `--seed-hex S` (`Seed`), или `--key-hex K [--param-hex P]` (`SeedWithKey`).

Настройка (в порядке `LoadConfig` главного окна): `--word-sep`, `--word-char-sep`; `--trigram ФАЙЛ` (`LoadTrigramFile`); `--wordlist ФАЙЛ [--min-word-len 1] [--max-word-len 100] [--lowercase-words]` (`LoadWordListFile`, значения по умолчанию — как в `Main.cpp:922`); `--charset STR [--ambig STR] [--symbols STR] [--include-each-subset] [--exclude-ambig] [--determine-subsets]` (`SetupCharSets`; если ничего из этого не задано — набор по умолчанию из конструктора).

Режимы `--mode`:

| режим | вызов | параметры |
|---|---|---|
| `password` (по умолч.) | `GetPassword` | `--length N` (16), `--flags` (PASSW_FLAG_*) |
| `phonetic` | `GetPhoneticPassw` | `--length`, `--flags` |
| `passphrase` | `GetPassphrase` | `--words N` (4), `--chars STR` (символы для комбинирования), `--flags` (PASSPHR_FLAG_*) |
| `format` | `GetFormatPassw` | `--format STR`, `--format-passw STR` (подстановка для `P`), `--flags` (PASSFORMAT_FLAG_*) |
| `main` | цикл `TMainForm::GeneratePassw` | см. ниже |

`--count N` — N паролей подряд одним экземпляром. Флаги передаются как есть (сырое значение).

Режим `main` повторяет ядро `Main.cpp:2220–2700`: символы → слова → формат → скрипт, с теми же буферами, автоматическими поправками флагов (`CHECKDUPLICATESBYSET`, фонетический регистр, снятие `FIRSTCHARNOTLC` при комбинировании), учётом энтропии (с ограничением 256 бит), `toupper` первого символа. Параметры: `--length N` (символы), `--words N` (слова), `--format STR`, `--passw-flags`, `--passphr-flags`, `--format-flags`, `--combine`, `--first-char-not-lc`, `--passphr-len SPEC` (поле «Specify length», напр. `>=20`, `10-30`, `*25`), `--exclude-duplicates`, `--check-each`, `--script ФАЙЛ`, `--advanced-flags N` (значение PASSWOPTION_*, передаётся в `init()` скрипта), `--dest N`.

Вывод: сведения о генераторе (`custom_charset`, `custom_charset_type`, `custom_charset_entropy`, `custom_charset_unique_size`, `word_list_size`, `word_list_entropy`, `phonetic_entropy`, `setup_charsets_result`, результаты загрузки файлов), `passwords` (`password`, `len` — возвращённая длина в символах UTF-32, для формата `security`, `passw_used`, `invalid_spec`; для `main` ещё `retries_before`, `duplicates_before`, `floor_entropy_bits`), `rng_bytes_consumed` (сколько байт взято у генератора), `prng_next_bytes` (следующие 8 байт потока — для проверки, что порт расходует ГСЧ так же).

```
run.sh gen --seed-passw test --charset "<AZ><az><09>" --length 16 --count 3
… "passwords":[{"len":16,"password":"np9RmVj2NiM2xl6y"},{"len":16,"password":"twnhPgDPsKtOuTVt"},{"len":16,"password":"6sGaWiQXT3iP5zjV"}],"rng_bytes_consumed":49,"prng_next_bytes":"147e2606b387f482"}
```

### `charsets`, `estimate`

```
run.sh charsets --charset "<AZ><az><09><sym>" --exclude-ambig [--perm-set-size 62 --perm-samples 16] [--word-index 0]
run.sh estimate --passw "Tr0ub4dor&3"     → {"security":68.983483887474947,…,"floor_entropy_bits":68}
```

`charsets` — только настройка генератора (+ `CalcPermSetEntropy`, `GetWord`). `estimate` — `PasswordGenerator::EstimatePasswSecurity`.

### `crypt-text-enc`, `crypt-text-dec` — `crypto/CryptText.cpp`

```
run.sh crypt-text-enc --passw secret --text "Hello" --rand-seed-hex 01
run.sh crypt-text-dec --passw secret --text "<base64>" [--version 0..3] [--old] [--codepage N]
```

Пароль передаётся как в `Main.cpp:1511–1567`: байты UTF-16LE без нуля; для версий 0 и 1 — строка, переведённая в однобайтовую кодировку (`--codepage`, по умолчанию `CP_ACP`). Источник случайности при шифровании (IV, дополнение) — `AESCtrPRNG::Seed(rand-seed)`. `--old` — перебор версий 0, 1, 2, как с флажком «PWGen <2.3.0». Буфер обмена подменён: результат (`text`) — то, что оригинал положил бы в буфер; `via_ansi`/`ansi_bytes` — если оригинал вызвал `SetClipboardTextBufAnsi` (шифрование всегда; расшифровка версий < 2). `result`/`result_name` — код `CRYPTTEXT_*`, `attempts` — все попытки.

### `db-create`, `db-dump` — `passw/PasswDatabase.cpp`

```
run.sh db-create --spec spec.json --out file.pwdb [--digest]
run.sh db-dump --in file.pwdb --passw ПАРОЛЬ [--keyfile ФАЙЛ] [--utf16-passw] [--header-only]
```

Спецификация (все поля необязательны, кроме `password`):

```json
{
  "password": "master", "passw_utf8": true, "keyfile": "key.hex",
  "recovery_password": "rec", "recovery_keyfile": null,
  "cipher": 0, "kdf_iterations": 1000, "compressed": true, "compression_level": 6,
  "rand_seed_hex": "0102",
  "default_user_name": "u", "passw_format_seq": "{8x}", "description": "Test",
  "default_passw_expiry_days": 90, "default_max_passw_history_size": 5,
  "master_passw_expiry_date": {"y": 2027, "m": 1, "d": 31},
  "entries": [{
    "title": "E", "user_name": "u", "password": "p", "url": "", "keyword": "", "notes": "a\r\nb",
    "key_values": [["k", "v"]], "tags": ["b", "a"],
    "creation_time": 133000000000000000, "modification_time": 133000000100000000,
    "passw_change_time": 0, "passw_expiry_date": "2026-12-31",
    "history": {"active": true, "max_size": 3,
                "entries": [{"time": 132000000000000000, "password": "old"}]}
  }]
}
```

Порядок действий как в приложении: `IsPasswUtf8`, `CipherType`, `KdfIterations` (до `New`, чтобы ключ сразу выводился с нужным числом итераций — результат тот же, что у `ChangeMasterKey`), ключ = `CombineKeySources(GetPasswBinary(utf8), keyfile)`, `New`, настройки базы, записи (`NewDbEntry` + поля; времена — FILETIME как u64, по умолчанию 0; дата истечения — число `(год<<16)|(месяц<<8)|день`, объект или `"ГГГГ-ММ-ДД"`; `user_name` не задан → имя по умолчанию базы, как в `NewDbEntry`; история перечисляется от новых к старым, `active:false` выставляется после добавления элементов), `SetRecoveryKey`, `Save`. Относительные пути ключевых файлов — от каталога спецификации. `rand_seed_hex` (или общий `--det-rand-seed`) делает файл воспроизводимым; без него соли/IV настоящие случайные (работает настоящий `RandomPool`).

`db-create` выводит размер, SHA-256 и hex файла (без `--digest`, если ≤ 1 МиБ), а также `randpool_log` (в детерминированном режиме: 32 — ключ шифрования паролей в памяти, 16 — соль хешей в памяти, 32 — соль файла, [32 — новый мастер-ключ, 32+32 — соли ячеек восстановления], IV, байты выравнивания).

`db-dump`: открывает базу как приложение (`Open(пусто, файл, true)` → `IsPasswUtf8` → ключ → `Open`), пароль восстановления подходит так же, как мастер-пароль (оригинал сам пробует обе ячейки; флаг `--recovery` принимается и ни на что не влияет). Выводит: `header` (сырые поля открытого заголовка), `layout` (для версии 1.8: блок восстановления, соль, IV, длина шифртекста, HMAC), `db` (все свойства базы и `check_master_key`), `entries` (все поля, `key_values`, `tags` в порядке `std::set`, времена u64, история; `strings_locale` — строки дат через `GetDateFormatEx` под wine, зависят от локали).

### `keyfile`, `keyfile-create`

```
run.sh keyfile --in key.bin                         → {"key":"…"}  (GetKeyFromKeyFile)
run.sh --det-rand-seed 01 keyfile-create --out k    → {"content":"65811BB1…","key":"…","randpool_log":[…]}
```

### `trigram-create` — `PasswordGenerator::CreateTrigramFile`

```
run.sh trigram-create --src words.txt --dst tri.bin
→ {"sigma":105,"entropy":2.125352394763857,"entropy_bits":"400100b8c19fe36d","file_size":70316,"file_sha256":"…","load_result":1,…}
```

### `mppg` — MP Password Generator (`MPPasswGen.cpp:286–470`, переписано в харнессе на исходных примитивах)

```
run.sh mppg --passw master --param example.com --charset-index 0 --length 16 [--add-length]
→ {"key":"48cab186…","passw_hash":{"dec99":"10","dec9999":"4610","hex16":"b833","hex32":"b833fc12"},"param_data":"6500…","password":"zOja7EN7O2F9RbfN","bits":95.267140966189999,…}
run.sh mppg --passw master --param example.com --hashapass   → {"password":"ZdOyfVmH",…}
```

`passw_hash` — все четыре вида контрольной суммы («Show password hash»).

### `zxcvbn` — `zxcvbn/zxcvbn.cpp`

```
run.sh zxcvbn --passw correcthorsebatterystaple → {"entropy":47.98…,"floor_entropy_bits":47,"matches":[{"begin":0,"length":7,"type":2,…},…]}
```

Как в `Main.cpp:3698`: пароль в UTF-8, без пользовательского словаря. Проверка по `common_passwords.txt` (делается в главном окне до zxcvbn) не воспроизводится.

### `lua` — `util/Scripting.cpp`

```
run.sh lua --script upstream/script_examples/passphrase_mixedcase.lua --seed-passw x --length 12 --words 3 --count 3
```

Синоним `gen --mode main --script ФАЙЛ`. Скрипт и генератор паролей используют один и тот же `AESCtrPRNG` (как при включённом детерминированном генераторе). `init()` вызывается перед первым `generate()`, после генерации первого базового пароля — в том же порядке, что в потоке `TScriptingThread`. Скрипт выполняется синхронно (оригинал ждёт результата каждого вызова, порядок обращений к ГСЧ тот же). Если `script_flags = 1` — базовая генерация отключается.

## Как собирается оригинальный код

- **Неизменённые исходники.** C-файлы (polarssl, chacha, blake2, minilzo, miniz, Lua, diceware) компилируются прямо из `upstream/`. C++-файлы ядра копируются байт в байт в `.ref/refharness/src/` и компилируются оттуда: clang всегда ищет `#include "…"` сначала рядом с файлом, а некоторые соседние заголовки (`Main.h`, `Util.h`, …) нужно подменить. Компилируются: `CryptText`, `CryptUtil`, `PasswGen`, `PasswDatabase`, `AESCtrPRNG`, `RandomGenerator`, `FastPRNG`, `RandomPool`, `EntropyManager`, `MemUtil`, `UnicodeUtil`, `StringFileStreamW`, `hrtimer`, `Scripting`, `zxcvbn`, Lua (кроме `lua.c`/`luac.c`).
- **`__property`** (в clang нет): заголовки с ним (`PasswGen.h`, `PasswDatabase.h`, `StringFileStreamW.h`, `EntropyManager.h`, `Scripting.h`) механически переписываются скриптом `gen_props.py` в `.ref/refharness/gen/` (`__declspec(property)` + публичные функции доступа; номера строк сохранены через `#line`). Остальной текст заголовков не меняется.
- **`__finally` / `__try` / `__except`** — макросы в `shim/prelude.h` (подключается ко всем C++-единицам через `-include`): `try {A} __finally {B}` → `try {A} catch(...) {сохранить} for(...) {B; перебросить}`. Ограничение: если A выходит по `return`, B не выполняется (в компилируемом коде таких мест нет). `__except` → `catch` по несуществующему типу (код SEH-фильтра есть только в `TScriptingThread::Execute`, харнесс его не запускает).
- **`std::basic_string<word32>`**: в libc++ ≥ 19 нет общего `char_traits`; специализация `char_traits<unsigned int>` (обычное беззнаковое сравнение) — в `shim/prelude.h`.
- **`_lrotl` в `chacha.c`**: в C++Builder виден через `<string.h>`; здесь C-файлы компилируются с `-include stdlib.h`.
- **Определения** из `PwTech.cbproj` (Release): `NDEBUG`, `LUA_INT_TYPE=1` (`luaconf.h` всё равно переопределяет его на `LUA_INT_DEFAULT`, т. е. 64-битные целые — как и в оригинале), плюс `UNICODE`, `_UNICODE`, `NOMINMAX`. Флаги: `-O2 -ffp-contract=off` (без FMA, все операции double округляются как SSE2-код оригинала).
- **`RandomPool`**: компилируется настоящий `RandomPool.cpp`, но с `-DGetInstance=GetInstance_real`; `RandomPool::GetInstance()` для остальных единиц определён в `shim/randpool/` и возвращает либо настоящий пул, либо (после `--det-rand-seed`/`rand_seed_hex`) подкласс `RandomPool`, у которого виртуальные `GetData`/`GetByte` берут байты из `AESCtrPRNG` и записываются в журнал.

### Подменённые заголовки приложения (`shim/app/`)

- `Main.h` — только `g_sExePath` (каталог exe с `\` на конце, как в оригинале), `g_config`, `g_sNewline` (`"\r\n"`).
- `Configuration.h` — `Configuration { FileEncoding = ceUtf8 }`.
- `Language.h` — `TRL()`/`TRLFormat()` без перевода (английские строки).
- `Util.h` — `SplitStringBuf`, `FloorEntropyBits`, `CppStdExceptionToString`, `OutOfDiskSpaceError` скопированы из `Util.cpp` дословно; функции буфера обмена пишут в переменную процесса (`SetClipboardTextBufAnsi` переводит в UTF-16 через `CP_ACP`, как оригинал).

### Заменитель VCL/RTL (`shim/vcl/`) и его отличия от настоящего

- `UnicodeString`/`AnsiString`: индексация с 1 и проверкой диапазона (`ERangeError`), `Length`, `c_str` (никогда не `nullptr`), `SetLength` (новые символы — нули; в VCL — мусор), `SubString`/`Pos`/`Delete`/`Insert` с семантикой Delphi `Copy`/`Pos`/`Delete`/`Insert`, `Trim` удаляет символы `<= ' '`. Нет copy-on-write (на результат не влияет). Преобразования `char*` ↔ `UnicodeString` и `AnsiString` ↔ `UnicodeString` — через `CP_ACP`, как в RTL.
- `LowerCase`/`UpperCase` (свободные функции) меняют только ASCII `A–Z`, как `System.SysUtils.LowerCase`; `AnsiLowerCase`/`AnsiUpperCase` и методы `UnicodeString::LowerCase()` — через `CharLowerBuffW` (wine).
- `ExtractFilePath`/`ExtractFileName`/`ExtractFileExt`: разделители только `\` и `:` (как в RTL под Windows).
- `TFileStream`: `CreateFileW` с теми же правами доступа и разделения, что `FileOpen`/`FileCreate` Delphi (`fmShareCompat` = без разделения); исключения `EFOpenError`/`EFCreateError` (текст сообщения отличается от RTL), `Read`/`Write` возвращают число байт.
- Иерархия исключений повторяет Delphi (`EStreamError` → `EFileStreamError` → `EFOpenError`…, `EOutOfMemory`, `ERangeError` …); `Exception` не наследует `std::exception`, как и в C++Builder.
- **`Log2`** — не `std::log2`, а точная копия RTL оригинала: `Ln(x) * 1.4426950408889634`, где `Ln` — собственная реализация RTL (восстановлена по дизассемблированию `PwTech.exe`, см. `vclshim.cpp`). Важно: так `Log2(8192) = 12.999999999999998`, `Log2(64) = 5.999999999999999`.
- **Оригинал собран с библиотеками Dinkumware** (C и C++, статически; в exe есть строки «Dinkumware», «vector<T> too long»), харнесс — с libc++ и `ucrtbase` wine. Где это меняет результат, в `PasswGen.cpp` и `zxcvbn.cpp` через `-include shim/dinkum_compat.h` подставлено поведение Dinkumware:
  - **`log()`** — восстановленная по дизассемблированию реализация Dinkumware `_Log` (`shim/dinkum_compat.cpp`); вызовы с константным аргументом сворачиваются компилятором, как и в оригинале (там `log(2.0)` хранится константой `0x3FE62E42FEFA39EF`). Обе реконструкции (`Ln` и `_Log`) укладываются в 1 ULP от правильно округлённого результата; запустить оригинал для побитной сверки нельзя, поэтому это реконструкция, а не проверенный факт;
  - **`std::sort`** (единственный вызов — сортировка подмножеств по частоте в `ParseCharSet`): Dinkumware сортирует до 32 элементов вставками, т. е. устойчиво, а libc++ для таких типов уже с 7 элементов делит массив неустойчиво; при равных частотах порядок, а значит и пароли, разошлись бы. Подставлена устойчивая сортировка для ≤ 32 элементов (больше — не воспроизводится).
- Остальная математика (`pow`, `exp` в zxcvbn; вся математика и `string.format` Lua) и прочие функции C (`toupper`, `isdigit` для кодов > 255) берутся из `ucrtbase` wine, а не из Dinkumware — возможны расхождения в последнем бите / на экзотических входах.
- `Format()` — упрощённый (`%d %u %x %s %f %%`, ширина и точность); используется только в сообщении об ошибке Lua.
- `TThread`/`TEvent` — тонкая обёртка над Win32, нужна только для компиляции `Scripting.cpp`.
- Не воспроизводятся: GUI, настоящий буфер обмена, переводы, `EntropyManager` как источник энтропии от мыши/клавиатуры (класс компилируется, но события не поступают), проверка по списку распространённых паролей.

## Файлы

```
build.sh, Makefile      сборка
run.sh                  запуск под wine
selftest.sh             проверка
harness.cpp             команды
harness_util.h          JSON, hex, UTF-8/16, разбор параметров
gen_props.py            __property → __declspec(property)
shim/prelude.h          char_traits<word32>, __finally/__try/__except
shim/dinkum_compat.*    log() и std::sort как в Dinkumware
shim/vcl/               заменитель VCL/RTL (vcl.h, System.hpp, Classes.hpp, … → vclshim.h)
shim/app/               Main.h, Language.h, Configuration.h, Util.h для ядра
shim/randpool/          переключатель RandomPool::GetInstance()
```
