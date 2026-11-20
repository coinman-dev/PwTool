# Сверка Rust-порта с оригинальным ядром Password Tech 3.6.1

Сгенерировано `tools/refcompare/compare.py` (2026-11-20 12:56, 935 с). Эталон — `tools/refharness` (неизменённые исходники оригинала под wine), порт — `crates/pwtech-core/examples/portharness.rs`. Входные данные детерминированы (seed 20261120).

Повторить: `python3 -I tools/refcompare/compare.py` (опции: `-j N`, `--area a,b`, `--no-build`).

Статусы: **match** — все общие поля совпали; **both-error** — обе стороны вернули ошибку (классы исключений совпали или несравнимы); **skipped** — вход не представим в API порта; **mismatch** — расхождение.

| область | случаев | match | both-error | skipped | mismatch | сравнено полей |
|---|---:|---:|---:|---:|---:|---:|
| prng | 42 | 41 | 0 | 0 | 1 | 997 |
| hash | 63 | 63 | 0 | 0 | 0 | 196 |
| chacha | 44 | 44 | 0 | 0 | 0 | 176 |
| charsets | 347 | 346 | 1 | 0 | 0 | 6686 |
| password | 176 | 154 | 19 | 0 | 3 | 3239 |
| phonetic | 100 | 98 | 0 | 0 | 2 | 1988 |
| passphrase | 206 | 189 | 1 | 0 | 16 | 4534 |
| format | 344 | 344 | 0 | 0 | 0 | 9539 |
| main | 166 | 143 | 23 | 0 | 0 | 4488 |
| lua | 51 | 45 | 6 | 0 | 0 | 1392 |
| estimate | 193 | 193 | 0 | 0 | 0 | 772 |
| estimate-nonbmp | 13 | 5 | 0 | 0 | 8 | 36 |
| zxcvbn | 125 | 120 | 0 | 0 | 5 | 2530 |
| mppg | 84 | 84 | 0 | 0 | 0 | 840 |
| crypt-enc | 33 | 33 | 0 | 0 | 0 | 255 |
| crypt-dec | 166 | 166 | 0 | 0 | 0 | 1591 |
| crypt-dec-old | 90 | 90 | 0 | 0 | 0 | 982 |
| keyfile | 16 | 14 | 1 | 0 | 1 | 48 |
| trigram | 10 | 5 | 3 | 0 | 2 | 40 |
| db-create | 19 | 19 | 0 | 0 | 0 | 114 |
| db-create-random | 1 | 1 | 0 | 0 | 0 | 3 |
| db-dump | 172 | 104 | 68 | 0 | 0 | 15940 |
| **всего** | 2461 | 2301 | 122 | 0 | 38 | 56386 |

Перекрёстное чтение баз: 20 проверок (дамп оригиналом файла порта = дамп оригиналом своего файла), расхождений: 0.

## Расхождения (38)

### #2 prng
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh prng-aesctr --seed-hex 01 --bytes 100
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 prng-aesctr --seed-hex 01 --bytes 100
```
- `/ok`: эталон 'False TIMEOUT: timeout after 120s', порт 'True : '

### #620 password
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 9fbb6f60bf3ef33ae09e7dd30b766250 --charset '<sym>' --length 300 --count 3 --flags 3170
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 9fbb6f60bf3ef33ae09e7dd30b766250 --charset '<sym>' --length 300 --count 3 --flags 3170
```
- `/ok`: эталон 'False TIMEOUT: timeout after 60s', порт 'True : '

### #667 password
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw eco --length 0 --flags 512
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw eco --length 0 --flags 512
```
- `/passwords[0]/password`: эталон 'exclude-ambig', порт ''

### #668 password
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw eco --length -1 --flags 512
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw eco --length -1 --flags 512
```
- `/ok`: эталон 'False NO_OUTPUT: wine: Unhandled page fault on read access to 0000000000000000 at address 00000001400929C0 (thread 02a0), starting debugger...\n02b0:fixme:dbghelp:elf_search_auxv can\'t find symbol in module\n02b0:fixme:dbghelp:elf_search_auxv can\'t find symbol in module\n02b0:fixme:dbghelp_dwarf:…(447 chars), порт 'True : '

### #745 phonetic
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 01 --mode phonetic --length 0
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 01 --mode phonetic --length 0
```
- `/passwords[0]/password`: эталон 'exclude-ambig', порт ''

### #746 phonetic
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 01 --mode phonetic --length -5
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 01 --mode phonetic --length -5
```
- `/ok`: эталон 'False std::exception: SecureMem: Size exceeds limit', порт 'True : '

### #854 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --key-hex 6b6579 --param-hex 70 --word-sep - --word-char-sep-u16hex e400 --mode passphrase --words 0 --flags 44 --count 3 --chars abc
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --key-hex 6b6579 --param-hex 70 --word-sep - --word-char-sep-u16hex e400 --mode passphrase --words 0 --flags 44 --count 3 --chars abc
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #861 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 24d8d3df93c39d704c6012b4634d9761a043 --word-sep-u16hex 2000 --word-char-sep-u16hex e400 --mode passphrase --words 0 --flags 33 --count 3 --chars 'X1!'
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 24d8d3df93c39d704c6012b4634d9761a043 --word-sep-u16hex 2000 --word-char-sep-u16hex e400 --mode passphrase --words 0 --flags 33 --count 3 --chars 'X1!'
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #863 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 67880461c084 --word-char-sep-u16hex 3dd800de --mode passphrase --words 0 --flags 0 --count 3 --chars 0123456789
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 67880461c084 --word-char-sep-u16hex 3dd800de --mode passphrase --words 0 --flags 0 --count 3 --chars 0123456789
```
- `/passwords[0]/password`: эталон <uninitialized memory, 236 characters>, порт ''
- `/passwords[1]/password`: эталон <uninitialized memory, 236 characters>, порт ''
- `/passwords[2]/password`: эталон <uninitialized memory, 236 characters>, порт ''

### #869 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 6a963bc53c93e41686 --word-sep-u16hex 2000 --word-char-sep-u16hex 20002b002000 --wordlist .ref/refcompare/files/wl_big.txt --lowercase-words --mode passphrase --words 0 --flags 41 --count 3 --chars abc
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 6a963bc53c93e41686 --word-sep-u16hex 2000 --word-char-sep-u16hex 20002b002000 --wordlist .ref/refcompare/files/wl_big.txt --lowercase-words --mode passphrase --words 0 --flags 41 --count 3 --chars abc
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #874 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --key-hex 6b6579 --param-hex 70 --word-sep - --word-char-sep .. --wordlist .ref/refcompare/files/wl_dups.txt --mode passphrase --words 0 --flags 33 --count 3 --chars-u16hex e4003dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --key-hex 6b6579 --param-hex 70 --word-sep - --word-char-sep .. --wordlist .ref/refcompare/files/wl_dups.txt --mode passphrase --words 0 --flags 33 --count 3 --chars-u16hex e4003dd800de
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #886 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw 'Pässwörd' --word-sep .. --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 9 --count 3
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw 'Pässwörd' --word-sep .. --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 9 --count 3
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #890 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw x --word-sep-u16hex e400 --word-char-sep-u16hex 20002b002000 --wordlist .ref/refcompare/files/wl_big.txt --mode passphrase --words 0 --flags 48 --count 3 --chars abc
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw x --word-sep-u16hex e400 --word-char-sep-u16hex 20002b002000 --wordlist .ref/refcompare/files/wl_big.txt --mode passphrase --words 0 --flags 48 --count 3 --chars abc
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #891 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw 'Pässwörd' --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_big.txt --mode passphrase --words 0 --flags 1 --count 3 --chars abc
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw 'Pässwörd' --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_big.txt --mode passphrase --words 0 --flags 1 --count 3 --chars abc
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #895 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw x --word-sep - --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_big.txt --lowercase-words --mode passphrase --words 0 --flags 17 --count 3 --chars-u16hex e4003dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw x --word-sep - --word-char-sep-u16hex e400 --wordlist .ref/refcompare/files/wl_big.txt --lowercase-words --mode passphrase --words 0 --flags 17 --count 3 --chars-u16hex e4003dd800de
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #901 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw-u16hex 3f04300440043e043b044c043dd800de --word-sep-u16hex 2000 --word-char-sep -- --mode passphrase --words 0 --flags 9 --count 3 --chars-u16hex e4003dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw-u16hex 3f04300440043e043b044c043dd800de --word-sep-u16hex 2000 --word-char-sep -- --mode passphrase --words 0 --flags 9 --count 3 --chars-u16hex e4003dd800de
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #904 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw seed1 --word-sep -- --word-char-sep-u16hex 2000 --wordlist .ref/refcompare/files/wl_utf8.txt --mode passphrase --words 0 --flags 24 --count 3
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw seed1 --word-sep -- --word-char-sep-u16hex 2000 --wordlist .ref/refcompare/files/wl_utf8.txt --mode passphrase --words 0 --flags 24 --count 3
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #911 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --key-hex 6b6579 --param-hex 70 --word-sep .. --word-char-sep _ --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 4 --count 3 --chars a
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --key-hex 6b6579 --param-hex 70 --word-sep .. --word-char-sep _ --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 4 --count 3 --chars a
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #931 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --key-hex 6b6579 --param-hex 70 --word-sep _ --word-char-sep .. --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 28 --count 3 --chars-u16hex e4003dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --key-hex 6b6579 --param-hex 70 --word-sep _ --word-char-sep .. --wordlist .ref/refcompare/files/wl_utf16.txt --mode passphrase --words 0 --flags 28 --count 3 --chars-u16hex e4003dd800de
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #943 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-hex 95eaff80b003b7 --word-sep-u16hex 3dd800de --word-char-sep _ --wordlist .ref/refcompare/files/wl_utf8.txt --mode passphrase --words 0 --flags 28 --count 3
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-hex 95eaff80b003b7 --word-sep-u16hex 3dd800de --word-char-sep _ --wordlist .ref/refcompare/files/wl_utf8.txt --mode passphrase --words 0 --flags 28 --count 3
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #944 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw seed1 --word-sep-u16hex e400 --mode passphrase --words 0 --flags 16 --count 3 --chars 0123456789
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw seed1 --word-sep-u16hex e400 --mode passphrase --words 0 --flags 16 --count 3 --chars 0123456789
```
- `/passwords[0]/password`: эталон '=<user>', порт ''
- `/passwords[1]/password`: эталон '=<user>', порт ''
- `/passwords[2]/password`: эталон '=<user>', порт ''

### #954 passphrase
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh gen --seed-passw x --word-sep _ --word-char-sep-u16hex 3dd800de --mode passphrase --words 0 --flags 34 --count 3 --chars a
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 gen --seed-passw x --word-sep _ --word-char-sep-u16hex 3dd800de --mode passphrase --words 0 --flags 34 --count 3 --chars a
```
- `/passwords[0]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[1]/password`: эталон 'chars-u16hex', порт ''
- `/passwords[2]/password`: эталон 'chars-u16hex', порт ''

### #1542 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 3dd800de3dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 3dd800de3dd800de
```
- `/security`: эталон '5.547368752524047 (401630816f2bc5bc)', порт '2.75 (4006000000000000)'
- `/floor_entropy_bits`: эталон 5, порт 2

### #1543 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 3dd800de61003dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 3dd800de61003dd800de
```
- `/security`: эталон '21.860914478074072 (4035dc64e427fb26)', порт '19.229419688230415 (40333abb3faa0216)'
- `/floor_entropy_bits`: эталон 21, порт 19

### #1718 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 3cd889df31006100620061003cd889df3cd889df3dd800de3100
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 3cd889df31006100620061003cd889df3cd889df3dd800de3100
```
- `/security`: эталон '58.416772080103236 (404d3558c99af6f3)', порт '43.641272487688965 (4045d2153785396e)'
- `/floor_entropy_bits`: эталон 58, порт 43

### #1719 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 3100620031003cd889df3dd800de61003dd800de3100
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 3100620031003cd889df3dd800de61003dd800de3100
```
- `/security`: эталон '47.32504270745974 (4047a99affdb2be3)', порт '39.64051664146686 (4043d1fc7305d26c)'
- `/floor_entropy_bits`: эталон 47, порт 39

### #1720 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 31003dd800de61003cd889df
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 31003dd800de61003cd889df
```
- `/security`: эталон '31.931568569324178 (403fee7b471b3a96)', порт '29.069712203742363 (403d11d8a8b3349e)'
- `/floor_entropy_bits`: эталон 31, порт 29

### #1721 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 6200610061003cd889df31003cd889df
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 6200610061003cd889df31003cd889df
```
- `/security`: эталон '36.99781553203574 (40427fb86b5ac465)', порт '32.79954695902241 (404066578e044f0a)'
- `/floor_entropy_bits`: эталон 37, порт 32

### #1722 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 620062003cd889df610062003dd800de3cd889df610031006200
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 620062003cd889df610062003dd800de3cd889df610031006200
```
- `/security`: эталон '52.68259471207783 (404a575f437665fe)', порт '43.31093484901009 (4045a7ccb68fd7af)'
- `/floor_entropy_bits`: эталон 52, порт 43

### #1725 estimate-nonbmp
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh estimate --passw-u16hex 62003dd800de6200610061003100310061003dd800de3dd800de
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 estimate --passw-u16hex 62003dd800de6200610061003100310061003dd800de3dd800de
```
- `/security`: эталон '48.93217220576991 (404877516b3902cf)', порт '38.76177571735136 (40436181dde0749d)'
- `/floor_entropy_bits`: эталон 48, порт 38

### #1742 zxcvbn
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh zxcvbn --passw 'P4$$w0rD!'
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 zxcvbn --passw 'P4$$w0rD!'
```
- `/matches[0]/entropy`: эталон '6.044394119358453 (40182d75a6eb1dfb)', порт '6.044394119358454 (40182d75a6eb1dfc)'
- `/matches[0]/mlt_entropy`: эталон '6.044394119358453 (40182d75a6eb1dfb)', порт '6.044394119358454 (40182d75a6eb1dfc)'

### #1755 zxcvbn
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh zxcvbn --passw-u16hex 540068006500200071007500690063006b002000620072006f0077006e00200066006f00780020006a0075006d007000730020006f00760065007200200074006800650020006c0061007a007900200064006f006700
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 zxcvbn --passw-u16hex 540068006500200071007500690063006b002000620072006f0077006e00200066006f00780020006a0075006d007000730020006f00760065007200200074006800650020006c0061007a007900200064006f006700
```
- `/matches[10]/entropy`: эталон '5.584962500721156 (4016570068e7ef5a)', порт '5.584962500721157 (4016570068e7ef5b)'
- `/matches[10]/mlt_entropy`: эталон '7.334962500721156 (401d570068e7ef5a)', порт '7.334962500721157 (401d570068e7ef5b)'

### #1762 zxcvbn
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh zxcvbn --passw abc123xyz789
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 zxcvbn --passw abc123xyz789
```
- `/matches[2]/entropy`: эталон '4.906890595608519 (4013a0a7eda4c113)', порт '4.90689059560852 (4013a0a7eda4c114)'
- `/matches[2]/mlt_entropy`: эталон '5.906890595608519 (4017a0a7eda4c113)', порт '5.90689059560852 (4017a0a7eda4c114)'

### #1775 zxcvbn
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh zxcvbn --passw '^3BMQ1Bnc'
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 zxcvbn --passw '^3BMQ1Bnc'
```
- `/matches[3]/entropy`: эталон '13.951831086888248 (402be756677b0098)', порт '13.951831086888252 (402be756677b009a)'
- `/matches[3]/mlt_entropy`: эталон '15.701831086888248 (402f6756677b0098)', порт '15.70183108688825 (402f6756677b0099)'

### #1810 zxcvbn
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh zxcvbn --passw-u16hex 610061006100f600f600360436043300f600320063003100
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 zxcvbn --passw-u16hex 610061006100f600f600360436043300f600320063003100
```
- `/matches[0]/entropy`: эталон '3.5849625007211556 (400cae00d1cfdeb3)', порт '3.5849625007211565 (400cae00d1cfdeb5)'
- `/matches[0]/mlt_entropy`: эталон '3.5849625007211556 (400cae00d1cfdeb3)', порт '3.5849625007211565 (400cae00d1cfdeb5)'

### #1947 keyfile
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh keyfile --in .ref/refcompare/files/missing.key
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 keyfile --in .ref/refcompare/files/missing.key
```
- `/exception`: эталон 'EFOpenError: Cannot open file "Z:\\<repo>\\.ref\\refcompare\\files\\missing.key". Файл не найден.', порт 'EStreamError: No such file or directory (os error 2)'

### #1958 trigram
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh trigram-create --src .ref/refcompare/files/wl_long.txt --dst .ref/refcompare/out/1958.ref
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 trigram-create --src .ref/refcompare/files/wl_long.txt --dst .ref/refcompare/out/1958.port
```
- `/exception`: эталон 'EStringFileStreamError: Invalid ANSI or UTF-8 character encoding, or Unicode string too long', порт 'EStreamError: Invalid ANSI or UTF-8 character encoding, or Unicode string too long'

### #1960 trigram
```
LC_ALL=ru_RU.UTF-8 tools/refharness/run.sh trigram-create --src .ref/refcompare/files/missing.txt --dst .ref/refcompare/out/1960.ref
cargo run -q --release -p pwtech-core --example portharness -- --acp 1251 trigram-create --src .ref/refcompare/files/missing.txt --dst .ref/refcompare/out/1960.port
```
- `/exception`: эталон 'EFOpenError: Cannot open file "Z:\\<repo>\\.ref\\refcompare\\files\\missing.txt". Файл не найден.', порт 'EStreamError: No such file or directory (os error 2)'

## Случаи, где обе стороны вернули ошибку (122)

- charsets #416: эталон `TIMEOUT: timeout after 120s`, порт `TIMEOUT: timeout after 120s`
- password #502: эталон `TIMEOUT: timeout after 60s`, порт `TIMEOUT: timeout after 60s`
- passphrase #915: эталон `TIMEOUT: timeout after 60s`, порт `TIMEOUT: timeout after 60s`
- main #1313: эталон `TIMEOUT: timeout after 90s`, порт `TIMEOUT: timeout after 90s`
- main #1323: эталон `HarnessError: Passphrase: Invalid length range.`, порт `HarnessError: Passphrase: Invalid length range.`
- lua #1514: эталон `ELuaError: Lua error: Runtime error
[string "lua_error.lua"]:2: boom`, порт `ELuaError: Lua error: Runtime error
lua_error.lua:2: boom
stack traceback:
	[C]: in ?
	[C]: in global 'error'
	lua_error.lua:2: in `
- keyfile #1943: эталон `EPasswDbError: Key file is empty`, порт `EPasswDbError: Key file is empty`
- trigram #1953: эталон `Exception: Source file does not contain enough trigrams`, порт `Exception: Source file does not contain enough trigrams`
- db-dump #2292: эталон `EPasswDbInvalidKey: Master key is invalid or file contents have been modified`, порт `EPasswDbInvalidKey: Master key is invalid or file contents have been modified`
- db-dump #2308: эталон `EPasswDbError: Invalid file size`, порт `EPasswDbError: Invalid file size`
- db-dump #2309: эталон `EPasswDbInvalidFormat: Unknown file format`, порт `EPasswDbInvalidFormat: Unknown file format`

