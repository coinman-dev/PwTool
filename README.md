# Password Tool (PwTool)

**Password Tool** — генератор и менеджер паролей для Linux. Это перенос [Password Tech](https://github.com/cthoeing/PasswordTech) 3.6.1 (автор — Christian Thöing, программа для Windows) на Rust и GTK4 с максимальным сохранением поведения оригинала: те же окна, алгоритмы с теми же результатами, совместимые файлы баз паролей, настроек и переводов.

Password Tool не связан с автором оригинала; ошибки порта сообщайте сюда: <https://github.com/coinman-dev/PwTool/issues>.

Состояние: перенесены генератор паролей со всеми окнами, менеджер паролей PassCube (файлы `.pwdb` совместимы с оригиналом в обе стороны), шифрование текста, MP-генератор, консольный режим, переводы (13 языков). План, опись функций и описание форматов — в `.ai/` (`PLAN.md`, `PARITY.md`, `FORMATS.md`); отличия от оригинала — `PLAN.md`, раздел 7.

## Структура

| Путь | Содержимое |
|---|---|
| `crates/pwtool-core` | ядро без интерфейса: криптография, генераторы случайных чисел, генерация паролей, базы паролей |
| `crates/pwtool` | программа (интерфейс GTK4 и консольный режим) |
| `upstream/` | исходники оригинала 3.6.1 (подмодуль git), используются как эталон |
| `tools/refharness/` | консольная сборка ядра оригинала под Windows для получения эталонных данных (запускается через wine) |
| `fixtures/` | эталонные файлы и результаты оригинала для тестов |
| `.ai/` | план работ и справочные материалы |

## Сборка

```sh
git submodule update --init
cargo build --release
cargo test
```

Для сборки нужны Rust 1.90+, компилятор C и пакет `libgtk-4-dev` (GTK 4.10 или новее).

## Установка

```sh
packaging/install.sh /usr/local          # сборка и установка (нужны права на запись в PREFIX)
packaging/build-deb.sh                   # пакет target/pwtool_<версия>_<архитектура>.deb
```

Установка кладёт программу в `PREFIX/bin/pwtool`, данные (переводы, список распространённых паролей, примеры скриптов, руководства в PDF) — в `PREFIX/share/pwtool/`, а также ярлык и значки. Руководства собираются из `upstream/manual/*.odt`, если установлен LibreOffice.

Настройки хранятся в `~/.config/pwtool/PwTool.ini` (формат `PwTech.ini` оригинала). Если рядом с программой лежит `PwTool.ini`, используется он (переносной режим, как в оригинале). «Restore Settings from Backup» принимает и резервную копию оригинала (`PwTech.ini`).

## Возможности, зависящие от рабочего стола

- Значок в трее — через StatusNotifierItem (KDE Plasma и другие окружения с поддержкой SNI).
- Автонабор и глобальные сочетания клавиш — через порталы `xdg-desktop-portal` (RemoteDesktop и GlobalShortcuts). При первом использовании рабочий стол спрашивает разрешение.
- Буфер обмена: пароли помечаются для Klipper как секретные и не попадают в его историю.

## Проверка

```sh
cargo test                                 # модульные тесты
tools/headless.sh env PWTOOL_AUTOMATE="click GenerateBtn;wait 500;print PasswBox;quit" ./target/debug/pwtool
python3 -I tools/refcompare/compare.py     # сверка с ядром оригинала под wine (нужен tools/refharness)
```

`tools/headless.sh` запускает программу в невидимом сеансе KWin со своей шиной D-Bus, окна на рабочем столе не появляются.

## Лицензия

GNU GPL v2 или более поздняя версия, как у оригинала (см. `LICENSE`). Password Tech: © 2002–2026 Christian Thöing. Сторонние компоненты оригинала — под своими лицензиями: miniLZO, miniz, zxcvbn-c (`crates/pwtool-core/csrc/`), Lua (через crate `mlua`).
