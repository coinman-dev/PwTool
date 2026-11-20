[English](README.md) | [Русский](README.ru_RU.md)

<p align="center">
  <img src="assets/about_logo.png" alt="Password Tool" width="160">
</p>

<h1 align="center">Password Tool</h1>
<p align="center">Криптографически стойкий генератор паролей и зашифрованный менеджер паролей для Linux и Windows.</p>

[![Релиз](https://img.shields.io/badge/release-0.1.0--beta-orange)](https://github.com/coinman-dev/PwTool/releases/tag/v0.1.0-beta)
[![Сборка](https://github.com/coinman-dev/PwTool/actions/workflows/release.yml/badge.svg)](https://github.com/coinman-dev/PwTool/actions/workflows/release.yml)
[![Linux](https://img.shields.io/badge/Linux-Ubuntu%2024.04%2B%20%2F%20Debian%2013%2B-blue)](#скачать)
[![Windows](https://img.shields.io/badge/Windows-10%20%2F%2011%20x64-blue)](#скачать)
[![Rust](https://img.shields.io/badge/Rust-1.90%2B-orange)](#сборка)
[![Лицензия](https://img.shields.io/badge/license-GPL--2.0--or--later-blue)](LICENSE)

**Password Tool** (PwTool) — перенос [Password Tech](https://github.com/cthoeing/PasswordTech) 3.6.1
на Rust и GTK4 для Linux и Windows с максимальным сохранением поведения оригинала: те же
окна, криптографические алгоритмы и совместимые форматы файлов. Базы паролей (`.pwdb`),
настройки (`PwTool.ini` / `PwTech.ini`), профили, списки слов, скрипты Lua и переводы
совместимы с оригинальным Password Tech.

<p align="center">
  <img src="doc/images/ru/main.png" alt="Password Tool: главное окно с генератором паролей и 16-сегментным индикатором стойкости">
</p>

> **0.1.0-beta — предварительный релиз.**
> Держите резервные копии важных баз `.pwdb` при тестировании бета-версий, а об ошибках
> сообщайте в [GitHub Issues](https://github.com/coinman-dev/PwTool/issues). Password Tool —
> независимый порт и не связан с автором Password Tech.

## Скачать

| Вариант | Скачать | Как использовать |
|---|---|---|
| Пакет для Linux | [pwtool_0.1.0.beta_amd64.deb](https://github.com/coinman-dev/PwTool/releases/download/v0.1.0-beta/pwtool_0.1.0.beta_amd64.deb) | `sudo apt install ./pwtool_0.1.0.beta_amd64.deb` ставит Password Tool, значки, переводы, примеры скриптов и руководства в PDF. |
| Windows, портативная версия | `pwtool_<версия>_windows_x64.zip` на [странице релизов](https://github.com/coinman-dev/PwTool/releases) (начиная со следующего релиза) | Распакуйте в любую папку и запустите `PwTool\bin\pwtool.exe`. GTK 4 и все библиотеки уже внутри, ничего не устанавливается. |

Linux: Ubuntu 24.04 и новее, Debian 13 и новее, amd64 (GTK 4.10+). Windows: 10 или 11, x64.
Рекламы и телеметрии нет.
Проверка обновлений обращается только к GitHub и отключается в **«Настройки» → «Параметры»**.
[Все релизы](https://github.com/coinman-dev/PwTool/releases) ·
[Изменения](CHANGELOG.ru_RU.md).

## Быстрый старт

1. Linux: установите `.deb`-пакет командой `sudo apt install ./pwtool_0.1.0.beta_amd64.deb`.
   Windows: распакуйте `.zip` в любую папку (например, `%LOCALAPPDATA%\Programs`).
2. Откройте **Password Tool** из меню приложений (**Служебные**) или запустите `pwtool`;
   в Windows запустите `PwTool\bin\pwtool.exe` (его можно закрепить в «Пуске» или на панели задач).
3. Выберите набор символов, список слов для парольной фразы, шаблон формата или скрипт Lua
   и нажмите **«Создать»** — 16-сегментная шкала покажет оценку энтропии в битах.
4. Нажмите значок базы на панели инструментов (или **«Файл» → «Менеджер паролей»**), чтобы
   открыть **PassCube** и хранить учётные данные в зашифрованной базе `.pwdb`.

Интерфейс доступен на 13 языках, поддерживает светлую, тёмную и системную темы, а также
выбор собственного шрифта интерфейса (с быстрым сбросом кнопкой «Установить по умолчанию»).

## Возможности

- **Криптографически стойкий ГСЧ**: пул энтропии на базе AES-256-CTR или ChaCha20 с
  подпиткой от ОС, событий ввода, таймера высокого разрешения и произвольных файлов;
  сохранение файла случайного зерна при выходе.
- **Гибкая генерация паролей**: наборы символов с весами частот, исключение неоднозначных
  символов, произносимые пароли по триграммам, парольные фразы по словарям, шаблоны формата
  и встроенные скрипты **Lua**.
- **Индикатор стойкости пароля**: 16 дискретных цветовых блоков с оценкой энтропии,
  проверкой по словарю распространённых паролей и анализом **zxcvbn**.
- **Менеджер паролей PassCube**: зашифрованные базы `.pwdb`, двусторонне совместимые с
  Password Tech 3.6.1 (мастер-пароль + опциональный ключевой файл, ключ восстановления,
  теги, уведомления об истечении срока, история паролей, произвольные поля «ключ–значение»,
  импорт/экспорт CSV, резервные копии).
- **Мастер-генератор (MP)**: детерминированная генерация паролей на основе мастер-пароля и
  параметров ресурса (включая режим Hashapass).
- **Шифрование текста**: шифрование и расшифровка текста или содержимого буфера обмена прямо
  с панели инструментов.
- **Профили и пакетная генерация**: сохранение пресетов генератора, генерация списков
  паролей в окно или файл, создание файлов случайных данных и таблиц триграмм.
- **Консольный режим**: генерация паролей, фраз и паролей по шаблону прямо из терминала без
  запуска графического окна.
- **Встроенная документация**: руководства `manual.pdf` и `scripting.pdf` ставятся вместе с
  программой и открываются из меню **«Справка»** или с панели инструментов.

## Интеграция с рабочим столом

Linux:

- **Область уведомлений (трей)**: через StatusNotifierItem (KDE Plasma и другие окружения
  с поддержкой SNI), сворачивание в трей и быстрая генерация из контекстного меню.
- **Автонабор и глобальные горячие клавиши**: через порталы `xdg-desktop-portal`
  (`RemoteDesktop` и `GlobalShortcuts`); при первом использовании система запросит
  разрешение.
- **Безопасный буфер обмена**: скопированные пароли помечаются как секретные
  (`x-kde-passwordManagerHint`), чтобы менеджеры буфера вроде Klipper не сохраняли их в
  истории; поддерживается автоматическая очистка по таймеру.

Windows (Win32 API, как в Password Tech):

- **Область уведомлений (трей)**: значок с тем же меню, сворачивание в трей и всплывающие
  уведомления.
- **Автонабор и глобальные горячие клавиши**: `SendInput` и `RegisterHotKey`, без запросов
  разрешения. Windows не даёт обычной программе вводить текст в окна программ, запущенных
  от имени администратора.
- **Безопасный буфер обмена**: пароли не попадают в журнал буфера обмена (Win+V), облачную
  синхронизацию буфера и программы-мониторы буфера.
- **Автозапуск** через ключ `Run` текущего пользователя; **«Поверх всех окон»**, единственный
  экземпляр, стиль **«По умолчанию»** следует светлому/тёмному режиму приложений Windows.

## Настройки и данные

| Что | Linux | Windows |
|---|---|---|
| Исполняемый файл | `/usr/bin/pwtool` | `PwTool\bin\pwtool.exe` |
| Ресурсы (переводы, словари, примеры скриптов, руководства PDF) | `/usr/share/pwtool/` | `PwTool\share\pwtool\` |
| Настройки, профили и файл случайного зерна | `~/.config/pwtool/PwTool.ini`, `randseed.dat` | `%APPDATA%\PwTool\PwTool.ini`, `randseed.dat` |
| Переносной режим | Положите `PwTool.ini` рядом с исполняемым файлом `pwtool` | Положите `PwTool.ini` рядом с `pwtool.exe` (в `bin`) |
| Базы паролей (`.pwdb`) | Там, куда вы их сохраните | Там, куда вы их сохраните |

Пункт **«Восстановить настройки из резервной копии»** принимает также резервные копии
оригинального Password Tech (`PwTech.ini`).

## Командная строка

```bash
pwtool --help
pwtool --gen 5
pwtool --profile "Мой профиль" --gen 10
pwtool --ini /path/to/PwTool.ini --readonly
pwtool --opendb ~/vault.pwdb
pwtool --silent
```

В Windows те же параметры работают с `pwtool.exe`; вывод появляется в окне командной строки
или PowerShell, из которого программа запущена (его можно перенаправить в файл).

## Сборка

Нужны Rust 1.90+, компилятор C/C++ (`build-essential`), `pkg-config` и `libgtk-4-dev`
(GTK 4.10 или новее). LibreOffice (`libreoffice-writer-nogui`) необязателен — используется
для сборки `manual.pdf` и `scripting.pdf` из `doc/manual/*.odt`.

```bash
cargo test --workspace --locked
cargo build --release --locked -p pwtool

packaging/install.sh /usr/local    # установка в PREFIX
packaging/build-deb.sh             # сборка target/pwtool_<версия>_<архитектура>.deb
```

### Сборка для Windows

Версия для Windows собирается кросс-компиляцией на Linux (инструменты Windows не нужны)
для цели Rust `x86_64-pc-windows-gnullvm` с помощью [llvm-mingw](https://github.com/mstorsjo/llvm-mingw)
(UCRT). GTK 4 и зависимости берутся из репозитория MSYS2 CLANG64, собранного таким же
набором инструментов; `packaging/windows/fetch-gtk.py` скачивает их (со сверкой SHA-256 по
базе репозитория) в `~/.cache/pwtool-win/sysroot`.

```bash
rustup target add x86_64-pc-windows-gnullvm
# llvm-mingw распакован в ~/.local/opt/llvm-mingw-<версия>-ucrt-ubuntu-22.04-x86_64
# (или задайте LLVM_MINGW); нужны python3 с zstd и glib-compile-schemas
packaging/windows/build.sh            # target/pwtool_<версия>_windows_x64.zip
packaging/windows/build.sh --tests    # плюс target/windows/tests/*.exe для запуска в Windows
```

Автоматизированная проверка интерфейса в виртуальной сессии:

```bash
tools/headless.sh env PWTOOL_AUTOMATE="click GenerateBtn;wait 400;print PasswBox;quit" ./target/debug/pwtool
```

## Автоматические релизы

Отправка тега версии, например `v0.1.0-beta`, запускает
[сборку релиза](.github/workflows/release.yml). Тег должен совпадать с `version` в
`Cargo.toml`. GitHub прогоняет тесты на Ubuntu 24.04, собирает PDF-руководства,
упаковывает и проверяет `pwtool_<версия>_amd64.deb`, кросс-компилирует
`pwtool_<версия>_windows_x64.zip`, запускает тесты и консольный режим Windows-сборки на
Windows-машине, считает `SHA256SUMS.txt` и публикует релиз. Теги с суффиксом через дефис автоматически помечаются как предварительные (prerelease).

## Лицензия

GNU General Public License v2 или более поздняя версия — см. [LICENSE](LICENSE).

Основано на **Password Tech** (Copyright © 2002–2026 Christian Thöing). Сторонние
компоненты сохраняют свои лицензии: miniLZO (GPL v2+), miniz (MIT), zxcvbn-c (MIT) и Lua
(MIT, через crate `mlua`). Сборка для Windows содержит GTK 4 и его зависимости в виде
неизменённых пакетов MSYS2 (в основном LGPL и MIT); их названия и версии перечислены в
`GTK-PACKAGES.txt` внутри архива, исходники доступны в [MSYS2](https://packages.msys2.org/).
