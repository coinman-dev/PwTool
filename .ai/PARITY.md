# Опись оригинала для сверки 1:1

Составлена по исходникам Password Tech 3.6.1 (`upstream/`) и выгруженным формам (`reference/forms/`). Каждая строка — то, что должно быть в порте. Отметка ставится, когда пункт сделан и проверен рядом с оригиналом.

Обозначения: `[ ]` не сделано, `[x]` сделано и проверено, `[~]` сделано с отклонением (см. `PLAN.md`, раздел 7), `[-]` убрано намеренно (см. `PLAN.md`, раздел 5.2).

Полные деревья элементов каждого окна с координатами — в `reference/forms/<Имя>.txt`, краткие — в `reference/forms-outline.txt`. Здесь перечислены функции, а не каждый элемент.

## A. Окна (21)

| | Форма | Заголовок | Объектов | Исходник (`upstream/src/main/`) | Этап |
|---|---|---|---|---|---|
| [ ] | Main | Password Tech | 185 | `Main.cpp` (4577 строк) | 5 |
| [ ] | PasswList | Password List | 13 | `PasswList.cpp` | 6 |
| [ ] | Progress | (ход выполнения) | 5 | `Progress.cpp` | 6 |
| [ ] | PasswOptions | Advanced Password Options | 28 | `PasswOptions.cpp` | 6 |
| [ ] | Configuration | Configuration | 84 | `Configuration.cpp` | 6 |
| [ ] | ProfileEditor | Profile Editor | 12 | `ProfileEditor.cpp` | 6 |
| [ ] | CharSetBuilder | Character Set Builder | 39 | `CharSetBuilder.cpp` | 6 |
| [ ] | PasswEnter | Password | 24 | `PasswEnter.cpp` | 6 |
| [ ] | MPPasswGen | MP Password Generator | 50 | `MPPasswGen.cpp` | 6 |
| [ ] | CreateRandDataFile | Create Random Data File | 9 | `CreateRandDataFile.cpp` | 6 |
| [ ] | CreateTrigramFile | Create Trigram File | 9 | `CreateTrigramFile.cpp` | 6 |
| [ ] | ProvideEntropy | Provide Additional Entropy | 5 | `ProvideEntropy.cpp` | 6 |
| [ ] | QuickHelp | Quick Help | 7 | `QuickHelp.cpp` | 6 |
| [ ] | InfoBox | (всплывающее сообщение) | 3 | `InfoBox.cpp` | 6 |
| [ ] | About | About | 12 | `About.cpp` | 6 |
| [ ] | PasswManager | PassCube | 156 | `PasswManager.cpp` (4166 строк) | 7 |
| [ ] | PasswMngDbSettings | Database Settings | 34 | `PasswMngDbSettings.cpp` | 7 |
| [ ] | PasswMngDbProp | Properties | 3 | `PasswMngDbProp.cpp` | 7 |
| [ ] | PasswMngPwHistory | Password History | 9 | `PasswMngPwHistory.cpp` | 7 |
| [ ] | PasswMngKeyValEdit | Key-Value List Editor | 4 | `PasswMngKeyValEdit.cpp` | 7 |
| [ ] | PasswMngColSelect | Select Columns | 4 | `PasswMngColSelect.cpp` | 7 |

## B. Главное окно

- [x] Панель кнопок: Password Manager, MP Password Generator, Profile Editor, шифрование буфера (левая кнопка мыши — зашифровать, правая — расшифровать), Clear Clipboard, Configuration, Help.
- [x] Выбор профиля, кнопки «Reload profile» и «Add profile».
- [x] Include characters: длина, набор символов (поле с историей), сведения о наборе, быстрая справка, кнопка «Build...».
- [x] Include words: число слов, файл словаря (поле с историей), сведения о словаре, обзор.
- [x] Combine words with characters; Specify length.
- [x] Format password: строка формата (поле с историей), быстрая справка, сведения.
- [x] Run script: файл скрипта (поле с историей), обзор, перезагрузка скрипта.
- [x] Multiple passwords: количество, кнопка Generate с меню (Generate to Clipboard, Generate to File, Include Header), кнопка Advanced с меню (Deactivate All Options, Deactivate All Starred Options).
- [x] Generated password: поле пароля, скрыть/показать, кнопка Generate, полоса качества, сведения об энтропии, перетаскивание пароля мышью.
- [x] Меню поля пароля: Undo, Cut, Copy, Encrypt & Copy, Paste, Perform Autotype, Add to Database, Delete, Select All, Save As File, Editable, Enable Password Testing, Change Font, Reset Font.
- [x] Строка состояния, индикатор энтропии пула и его меню (сброс счётчиков).
- [x] Заголовок окна с предупреждением при включённом детерминированном генераторе.
- [x] Сообщения при запуске: ошибки запуска, результат проверки обновлений.
- [-] Напоминание о пожертвовании при запуске.
- [x] Автоочистка поля пароля по таймеру (`AutoClearPassw`, 3–32767 с, по умолчанию 30).

## C. Меню и сочетания клавиш

Главное окно:

- [x] **File:** Profile (список профилей, Profile Editor `F10`) · Password Manager · Backup Settings · Restore Settings from Backup · Exit `Alt+Q`
- [x] **Tools:** Clear Clipboard `F2` · Encrypt Clipboard `F3` · Decrypt Clipboard `F4` · Create Random Data File `F5` · Create Trigram File `F6` · MP Password Generator `F8` · Deterministic Random Generator (Set up `Ctrl+F8`, Reset `Ctrl+F7`, Deactivate `F7`) · Provide Additional Entropy (As Text `F9`, From File `Ctrl+F9`)
- [x] **Options:** Configuration `F11` · Clear Password Cache · Always on Top · Hide Entropy Progress · Save Settings on Exit · Save Settings Now
- [x] **Help:** Open Manual `F1` · Command Line Arguments · Visit Website (репозиторий порта) · Get Translations (репозиторий порта) · Donate (страница автора оригинала) · Check for Updates (GitHub Releases порта) · Total Entropy Bits · Timer Info · About
- [-] **Help | Enter Donor Key**
- [x] **Меню трея:** Restore · Generate Password · Generate and Autotype Password · Generate and Show Password · Profile · Reset All Window Positions · Clear Clipboard · Encrypt Clipboard · Decrypt Clipboard · Create Random Data File · MP Password Generator · Password Manager · Open Manual · About · Exit
- [x] **Меню списка (поля с историей):** Undo, Cut, Copy, Paste, Delete, Select All, Remove Entry, Clear List

Password List:

- [x] Copy `Ctrl+C` · Encrypt & Copy `Ctrl+E` · Add to Database `Ctrl+D` · Select All `Ctrl+A` · Save As File `Ctrl+S` · Change Font · Reset Font

PassCube:

- [x] **File:** New `Ctrl+N` · Open `Ctrl+O` · Open As Read-only · Close `Ctrl+W` · Lock `Ctrl+L` · недавние файлы · Clear Recently Opened List · Save `Ctrl+S` · Save As · Export To → CSV File · Database Settings · Change Master Password · Set Recovery Password · Properties · Exit `Ctrl+Q`
- [x] **Edit:** Copy User Name · Copy Password · Open URL · Run · Perform Autotype · Add Entry · Duplicate Entry · Delete Entry · Rearrange Entry (Top, Up, Down, Bottom) · Select All
- [x] **Контекстное меню списка записей** (сочетания клавиш заданы здесь): Copy User Name `Ctrl+B` · Copy Password `Ctrl+C` · Open URL `Ctrl+U` · Run `Ctrl+R` · Perform Autotype `Ctrl+T` · Duplicate Entry `Ctrl+P` · Delete Entry `Ctrl+D` · Rearrange Entry · Select All `Ctrl+A`
- [x] **View:** Show Columns · Show Passwords in List · Sort Entries By (поле; Ascending / Descending) · Sort Tags By (Name / Frequency; Ascending / Descending) · Filter (Expired Entries / Entries that Will Expire Soon / Entries with Weak Passwords) · Change List Font · Change Password Font · Reset Fonts
- [x] **Меню поиска:** Case-sensitive · Fuzzy search · Select fields
- [x] Меню тегов, меню срока действия (содержимое строится в коде — `PasswManager.cpp`)

## D. Генерация паролей (`passw/PasswGen.cpp`, `main/Main.cpp:2085`)

- [x] Пароль из набора символов; до 11 кодов наборов: `<AZ>` `<az>` `<09>` `<Hex>` `<hex>` `<base64>` `<easytoread>` `<symbols>` `<brackets>` `<punctuation>` `<highansi>` и их краткие формы (`<b64>`, `<etr>`, `<sym>`, `<brac>`, `<punct>`, `<high>`).
- [x] Наборы с частотами подмножеств; фонетические `<phonetic>`, `<phoneticu>`, `<phoneticx>`.
- [x] Фраза из слов: встроенный словарь на 8192 слова или свой файл; разделители слов и «слово–символы».
- [x] Сочетание слов с символами; заданная общая длина.
- [x] Форматированный пароль: 22 набора (`x a A U E d h H l L u v V Z c C z p b s S y`), служебные спецификаторы `P q Q r W w`, повтор `[…]` (до 4 уровней), перестановка `{…}`, свой набор `<…>`, текст в кавычках, числа и диапазоны повторов.
- [x] Фонетические пароли по триграммам (встроенная таблица или файл).
- [x] Скрипт Lua: как дополнение к паролю и как самостоятельный генератор (`script_flags`).
- [x] Расчёт энтропии для каждого способа; ограничение полосы качества 128 битами.
- [x] Оценка стойкости введённого пароля: простой метод и zxcvbn; проверка по списку распространённых паролей.
- [x] Назначение результата (8): поле в окне, список в окне, список в буфер, список в файл, пароль в буфер, окно сообщения, консоль, автонабор.
- [x] Заголовок списка паролей (Include Header), ход выполнения с отменой.
- [x] Неоднозначные символы по умолчанию `B8G6I1l|0OQDS5Z2`, специальные символы по умолчанию `!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~`.

## E. Дополнительные параметры (`main/PasswOptions.h`, 19 флагов)

- [x] `EXCLUDEAMBIG` — исключить неоднозначные символы
- [x] `FIRSTCHARNOTLC` — первый символ не строчная буква
- [x] `DONTSEPWORDS` — не разделять слова
- [x] `DONTSEPWORDSCHARS` — не разделять слова и символы
- [x] `REVERSEWCHORDER` — обратный порядок слов и символов
- [x] `INCLUDEUPPERCASE`, `INCLUDELOWERCASE`, `INCLUDEDIGIT`, `INCLUDESYMBOL` — хотя бы один символ вида
- [x] `INCLUDESUBSET` — подмножества брать из своего набора
- [x] `EXCLUDEREPCHARS` — без повторов подряд
- [x] `EXCLUDEDUPLICATES` — без одинаковых паролей в списке
- [x] `LOWERCASEWORDS` — слова строчными
- [x] `EACHCHARONLYONCE`, `EACHWORDONLYONCE` — без повторов символов / слов
- [x] `CAPITALIZEWORDS` — слова с заглавной
- [x] `CHECKEACHPASSW` — проверять каждый пароль
- [x] `INCLUDE_CHAR_FROM_EACH_SUBSET` — символ из каждого подмножества
- [x] `REMOVEWHITESPACE` — убрать пробелы
- [x] Поля окна: переопределение неоднозначных и специальных символов, мин./макс. длина слов, файл триграмм, разделители слов и «слово–символы».
- [x] Меню списка параметров: Select All, Deselect All, Invert Selection.

## F. Инструменты

- [x] Шифрование и расшифровка текста в буфере обмена; флажок «PWGen <2.3.0» для старого формата.
- [x] Очистка буфера обмена; автоочистка (`AutoClearClip`, 3–32767 с, по умолчанию 10) только если в буфере всё ещё наш текст.
- [x] MP Password Generator: ввод мастер-пароля с подтверждением, контрольная сумма (4 вида), срок жизни ключа, параметр, 6 наборов символов, длина, «Add password length to parameter», совместимость с Hashapass, последовательность автонабора, «Use as default random generator».
- [x] Детерминированный генератор: настройка ключом, сброс, отключение.
- [x] Создание файла случайных данных заданного размера.
- [x] Создание файла триграмм из словаря.
- [x] Дополнительная энтропия: вводом текста и из файла.
- [x] Character Set Builder: строчные, прописные, цифры, символы, диапазон, дополнительные символы, «At least N» для каждого, исключаемые символы, тег-комментарий.
- [x] Profile Editor: добавить, загрузить, удалить, порядок, «Save Advanced password options for this profile», подтверждение.
- [x] Резервная копия настроек и восстановление из неё.
- [x] Timer Info, Total Entropy Bits, Clear Password Cache.
- [x] About: название, версия порта, авторство Christian Thöing и строка о порте, ссылки, сведения о переводе.
- [-] Отметка «DONOR» в окне About.

## G. Конфигурация (`main/Configuration.h`)

- [x] **General:** шрифт интерфейса, задержка автонабора (по умолчанию 50 мс), стиль интерфейса, значок программы (доступен всем, в оригинале — только с ключом донора), значок в трее постоянно, сворачивать в трей, сворачивать перед автонабором, спрашивать при выходе, запуск при входе в систему, загружать профиль при запуске.
- [x] **Security:** шифр пула случайных чисел (AES-CTR, ChaCha20, ChaCha8), проверка по списку распространённых паролей, zxcvbn, Benchmark с выбором объёма данных.
- [x] **Hot Keys:** до 10 сочетаний; действия: одиночный пароль, список паролей, пароль в буфер, пароль в окне сообщения, пароль автонабором, показать MP-генератор, показать менеджер паролей, поиск в базе, поиск в базе с автонабором; флажок «Show/restore main window».
- [x] **Files:** кодировка текстовых файлов, последовательность перевода строки.
- [x] **Updates:** ежедневно, еженедельно (по умолчанию), ежемесячно, выключено.
- [x] **Language:** выбор, установка и удаление файла перевода, преобразование `.lng` в `.po`.
- [x] **Database:** блокировка при сворачивании; блокировка по простою (по умолчанию 300 с); резервная копия перед сохранением; нумерация копий (по умолчанию до 5, максимум 999); открывать последнюю базу при запуске; очищать буфер при закрытии и блокировке; открывать окно при запуске; автосохранение (при изменении записи / при любом изменении); предупреждения об истёкших и истекающих записях (по умолчанию за 7 дней); список недавних файлов; последовательность автонабора по умолчанию.
- [x] Меню «Restore Default».

## H. PassCube (`passw/PasswDatabase.*`, `main/PasswManager.cpp`)

- [x] Создание, открытие, открытие только для чтения, сохранение, «сохранить как», закрытие с вопросом о несохранённых изменениях.
- [x] Мастер-пароль, ключевой файл, их сочетание; создание ключевого файла; запоминание пароля на N секунд; кнопка Skip вместо подтверждения.
- [x] Пароль восстановления: установка и снятие.
- [x] Смена мастер-пароля с отменой долгого вывода ключа; срок действия мастер-пароля с предупреждением.
- [x] Блокировка и разблокировка: вручную, по простою, при сворачивании.
- [x] Запись: Title, User name, Password, URL, Keyword, Notes, список «ключ–значение», теги, время создания, изменения и смены пароля, срок действия, история паролей.
- [x] Редактор записи: принять и отменить изменения, предыдущая и следующая запись, удалить, скрыть и показать пароль, правая кнопка — сгенерировать пароль, оценка качества, открыть URL, скрыть и показать заметки.
- [x] Список записей: столбцы на выбор, показ паролей, сортировка по столбцу, множественный выбор, перетаскивание для изменения порядка, перемещение в начало, вверх, вниз, в конец.
- [x] Теги: список с числом записей, фильтр по тегу, добавление существующего тега, сортировка по имени и частоте.
- [x] Поиск: строка поиска с историей, с учётом регистра, нечёткий, по выбранным полям; фильтры по сроку и слабым паролям.
- [x] Копирование имени и пароля в буфер, открытие URL, «Run», автонабор, перетаскивание полей мышью.
- [x] Database Settings: имя пользователя по умолчанию, формат паролей для новых записей с пробной генерацией, срок действия мастер-пароля, описание базы, сжатие с уровнем, шифр, число раундов вывода ключа и подбор под задержку в 1 секунду.
- [x] Properties, Password History (включение, размер, очистка, копирование), Key-Value List Editor.
- [x] Экспорт в CSV с выбором столбцов.
- [x] Резервные копии перед сохранением, автосохранение, список недавних файлов.
- [x] Сохранение базы при завершении сеанса системы.
- [x] Добавление пароля в базу из главного окна, списка паролей и MP-генератора.

## I. Командная строка (`PwTech.cpp`, `main/Main.h:61`)

- [x] `help` — показать справку
- [x] `ini <файл>` — другой файл настроек
- [x] `readonly` — не записывать настройки
- [x] `profile <имя>` — загрузить профиль
- [x] `gen [N]` — сгенерировать N паролей в консоль
- [x] `silent` — не показывать главное окно
- [x] `opendb <файл>` — открыть базу; аргумент без переключателя тоже считается файлом базы
- [x] Сообщение о неизвестных переключателях

## J. Файлы (подробности — `FORMATS.md`)

- [x] `.pwdb` — база паролей, чтение 1.0–1.8, запись 1.8
- [x] Зашифрованный текст (base64), версии 0–3
- [x] Ключевой файл: 32 байта, 64 шестнадцатеричных символа или любой файл
- [x] Файл триграмм
- [x] `randseed.dat` — зерно пула случайных чисел
- [x] `PwTech.ini` — настройки и профили
- [x] `.po` и `.lng` — переводы
- [x] Словари, `common_passwords.txt`, скрипты `.lua`
- [x] CSV-экспорт, списки паролей в текстовых файлах в выбранной кодировке

## K. Связь с системой

- [x] Значок в трее, сворачивание в трей, двойной щелчок — восстановить.
- [x] Глобальные сочетания клавиш (`RegisterHotKey` в оригинале).
- [x] Автонабор (`util/SendKeys.cpp`): поля записи `{title}` `{username}` `{password}` `{url}` `{keyword}` `{notes}` `{keyvaluelist}` `{tags}`, `{parameter}`; клавиши `tab return enter ctrl backspace clear shift alt pause capslock escape space pageup pagedown end home left right down up select print execute snapshot insert delete help`; пауза `{wait:N}`; задержка между символами; сворачивание окна перед набором.
- [x] Буфер обмена: защищённая запись, очистка, несколько попыток при занятом буфере.
- [x] Перетаскивание текста из полей в другие программы.
- [x] Единственный экземпляр программы: повторный запуск показывает окно работающей копии (вместо сообщения).
- [x] Автозапуск при входе пользователя.
- [x] Поведение при завершении сеанса: сохранение настроек и базы.
- [x] Always on Top — в KDE через KWin; без KWin пункт неактивен.
- [x] Запоминание размеров окон (положение на Wayland недоступно приложению).
- [x] Проверка обновлений в фоне через GitHub Releases репозитория порта.
- [x] Открытие руководства, сайта, страницы переводов и пожертвований.

## L. Локализация

- [x] 680 строк из `English.pot` используются без изменений как ключи.
- [x] Переводы `.po`: бразильский португальский, китайский упрощённый и традиционный, нидерландский, французский, немецкий, венгерский, японский, польский, русский, испанский, турецкий; `.lng`: греческий.
- [x] Проверка минимальной версии перевода (`PROGRAM_LANGVER_MIN` = 3.6.0).
- [x] Руководство на языке перевода, если оно указано в заголовке `.po`.
