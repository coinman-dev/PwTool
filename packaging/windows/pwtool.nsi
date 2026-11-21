; Installer of Password Tool for Windows x64 (NSIS 3, Unicode).
;
; Built on Linux or Windows by packaging/windows/build-installer.sh, which
; passes the version and the paths:
;   makensis -DVERSION=0.1.0-beta -DVERSION_NUMERIC=0.1.0.0 -DSOURCE_EXE=... \
;            -DROOT=... -DICON=... -DOUTPUT=... pwtool.nsi
;
; The user chooses between installing for everyone (the default for an
; administrator, into C:\Program Files\PwTool) and for the current account
; only (into %LOCALAPPDATA%\Programs\PwTool, no administrator rights needed).
; Administrator rights are requested only when the all-users variant is
; chosen: the manifest asks for an ordinary token, and the installer restarts
; itself through the `runas` verb when it needs more.
;
; The program keeps its settings in %APPDATA%\PwTool of each user (nothing is
; written into the installation directory: no PwTool.ini there means no
; portable mode), so the installation directory stays read-only.
;
; Silent use:  setup.exe /S [/AllUsers | /CurrentUser] [/D=directory]
;              uninstall.exe /S   (keeps the settings)

Unicode true
ManifestDPIAware true

!ifndef VERSION
    !error "VERSION is required; build through packaging/windows/build-installer.sh"
!endif
!ifndef VERSION_NUMERIC
    !define VERSION_NUMERIC "0.0.0.0"
!endif
!ifndef SOURCE_EXE
    !error "SOURCE_EXE is required; build through packaging/windows/build-installer.sh"
!endif
!ifndef ROOT
    !error "ROOT is required; build through packaging/windows/build-installer.sh"
!endif
!ifndef ICON
    !error "ICON is required; build through packaging/windows/build-installer.sh"
!endif
!ifndef OUTPUT
    !define OUTPUT "pwtool-setup.exe"
!endif

!define NAME "Password Tool"
!define SHORT_NAME "PwTool"
!define PUBLISHER "CoinMan.dev"
!define EXE "pwtool.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${SHORT_NAME}"
; The value the program itself writes and reads for «Запускать при старте
; системы» (pwtool_app::platform::autostart, PROGRAM_SHORT_NAME).
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define RUN_VALUE "PwTool"
; Files of the program in %APPDATA%\PwTool (PROGRAM_INIFILE,
; PROGRAM_RANDSEEDFILE). Other files there (languages and trigram files the
; user added, perhaps databases) are never deleted.
!define INI_FILE "PwTool.ini"
!define SEED_FILE "randseed.dat"

; ---------------------------------------------------------------- MultiUser

; `Standard` leaves `RequestExecutionLevel user` in the manifest, so Windows
; does not raise the installer before the user has chosen anything. The
; all-users half of MultiUser.nsh is normally tied to an elevating manifest,
; so it is switched on by hand and this script does the elevating itself.
!define MULTIUSER_EXECUTIONLEVEL Standard
!define MULTIUSER_EXECUTIONLEVEL_ALLUSERS
!define MULTIUSER_MUI
!define MULTIUSER_INSTALLMODE_COMMANDLINE
!define MULTIUSER_INSTALLMODE_INSTDIR "${SHORT_NAME}"
!define MULTIUSER_INSTALLMODE_INSTDIR_REGISTRY_KEY "Software\${SHORT_NAME}"
!define MULTIUSER_INSTALLMODE_INSTDIR_REGISTRY_VALUENAME "InstallDir"
!define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_KEY "${UNINSTALL_KEY}"
!define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_VALUENAME "UninstallString"
!define MULTIUSER_INSTALLMODE_FUNCTION InstallModeChanged
!define MULTIUSER_USE_PROGRAMFILES64

!include "MUI2.nsh"
!include "MultiUser.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "Sections.nsh"
!include "StrFunc.nsh"
!include "x64.nsh"

!insertmacro GetSize

Name "${NAME}"
OutFile "${OUTPUT}"
; Empty until MultiUser sets a default, or NSIS parses an explicit /D= path.
InstallDir ""
BrandingText "${NAME} ${VERSION}"
ShowInstDetails show
ShowUninstDetails show
SetCompressor /SOLID lzma

VIProductVersion "${VERSION_NUMERIC}"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${NAME} installer"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "Password Tech (c) 2002-2026 Christian Thoeing; port (c) 2026 CoinMan.dev; GPL-2.0-or-later"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunProgram
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_LANGDLL_ALLLANGUAGES
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "Software\${SHORT_NAME}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "InstallerLanguage"

!define MUI_PAGE_CUSTOMFUNCTION_PRE ResumeBeforeComponents
!insertmacro MUI_PAGE_WELCOME
!define MUI_PAGE_CUSTOMFUNCTION_PRE ResumeBeforeComponents
!insertmacro MUI_PAGE_LICENSE "${ROOT}/LICENSE"
; Checked when the install mode page is left, that is as soon as the choice
; between the two variants is known.
!define MULTIUSER_PAGE_CUSTOMFUNCTION_PRE ResumeBeforeComponents
!define MULTIUSER_PAGE_CUSTOMFUNCTION_LEAVE ElevateForAllUsers
!insertmacro MULTIUSER_PAGE_INSTALLMODE
; The install mode is chosen by now, so an earlier installation can be found.
!define MUI_PAGE_CUSTOMFUNCTION_PRE DetectUpgrade
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

; MultiUser.nsh has no install-mode page for the uninstaller: the mode comes
; from the `/AllUsers` or `/CurrentUser` argument written into
; UninstallString, and from the registry when it is started by hand.
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Russian first, so it is the default of the language dialog.
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "English"

; ----------------------------------------------------------------- strings

LangString DescMain ${LANG_RUSSIAN} "Программа (один файл, переводы и руководства встроены). Обязательный компонент."
LangString DescMain ${LANG_ENGLISH} "The program (one file, translations and manuals built in). Required."
LangString NameDesktop ${LANG_RUSSIAN} "Ярлык на рабочем столе"
LangString NameDesktop ${LANG_ENGLISH} "Desktop shortcut"
LangString DescDesktop ${LANG_RUSSIAN} "Создать ярлык на рабочем столе."
LangString DescDesktop ${LANG_ENGLISH} "Create a shortcut on the desktop."
LangString NameAutostart ${LANG_RUSSIAN} "Запускать при входе в систему"
LangString NameAutostart ${LANG_ENGLISH} "Start at logon"
LangString DescAutostart ${LANG_RUSSIAN} "Запускать программу в области уведомлений при входе в систему. Потом это можно изменить в Настройки → Параметры."
LangString DescAutostart ${LANG_ENGLISH} "Start the program in the notification area when you log in. You can change this later in Options → Configuration."
LangString Need64 ${LANG_RUSSIAN} "Программа работает только на 64-разрядной Windows."
LangString Need64 ${LANG_ENGLISH} "The program only runs on 64-bit Windows."
LangString ProgramRunning ${LANG_RUSSIAN} "Password Tool запущен.$\r$\n$\r$\nСохраните открытую базу паролей и закройте программу (Файл → Выход или меню значка в области уведомлений), затем нажмите «Повтор»."
LangString ProgramRunning ${LANG_ENGLISH} "Password Tool is running.$\r$\n$\r$\nSave the open password database and exit the program (File → Exit or the menu of its notification area icon), then click Retry."
LangString AskRemoveData ${LANG_RUSSIAN} "Удалить также настройки программы (${INI_FILE} и ${SEED_FILE}) из папки:$\r$\n$0$\r$\n$\r$\nБазы паролей (.pwdb) и другие ваши файлы не удаляются.$\r$\nЕсли вы собираетесь установить программу заново, выберите «Нет»."
LangString AskRemoveData ${LANG_ENGLISH} "Also delete the settings of the program (${INI_FILE} and ${SEED_FILE}) from the folder:$\r$\n$0$\r$\n$\r$\nPassword databases (.pwdb) and your other files are not deleted.$\r$\nChoose No if you are going to install the program again."
LangString KeptFiles ${LANG_RUSSIAN} "В папке остались ваши файлы (переводы, файлы триграмм или базы паролей):"
LangString KeptFiles ${LANG_ENGLISH} "Your files are left in the folder (languages, trigram files or password databases):"
LangString NeedAdmin ${LANG_RUSSIAN} "Для установки сразу для всех пользователей нужны права администратора.$\r$\n$\r$\nРазрешите запрос Windows или выберите «Только для меня»."
LangString NeedAdmin ${LANG_ENGLISH} "Installing for all users needs administrator rights.$\r$\n$\r$\nAllow the Windows prompt, or choose “Only for me”."
LangString NeedAdminUninstall ${LANG_RUSSIAN} "Для удаления программы, установленной для всех пользователей, нужны права администратора."
LangString NeedAdminUninstall ${LANG_ENGLISH} "Removing a program installed for all users needs administrator rights."

; ---------------------------------------------------------------- helpers

; A running program locks its executable. It is not killed: it may hold a
; password database with unsaved changes, so the user closes it. The check
; opens the executable for writing, which fails while it runs. A silent run
; gives up with ERROR_SHARING_VIOLATION.
!macro WaitForProgram UN
Function ${UN}WaitForProgram
    ${IfNot} ${FileExists} "$INSTDIR\${EXE}"
        Return
    ${EndIf}
    retry:
    ClearErrors
    FileOpen $R0 "$INSTDIR\${EXE}" a
    ${IfNot} ${Errors}
        FileClose $R0
        Return
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(ProgramRunning)" /SD IDCANCEL IDRETRY retry
    SetErrorLevel 32
    Quit
FunctionEnd
!macroend
!insertmacro WaitForProgram ""
!insertmacro WaitForProgram "un."

; With an `asInvoker` manifest the token of an administrator is filtered, and
; `UserInfo::GetAccountType` answers "User" — which makes MultiUser.nsh skip
; the install mode page and silently pick the per-user variant. The original
; account type sees through the filter, and the choice comes back. Rights
; themselves are still requested only when the all-users variant is chosen.
Function RestoreAdminPrivileges
    ${If} $MultiUser.Privileges == "Admin"
    ${OrIf} $MultiUser.Privileges == "Power"
        Return
    ${EndIf}
    UserInfo::GetOriginalAccountType
    Pop $0
    ${If} $0 != "Admin"
        Return
    ${EndIf}
    StrCpy $MultiUser.Privileges "Admin"

    ; Repeat the choice MULTIUSER_INIT_CHECKS makes for an administrator:
    ; all users, unless only a per-user installation exists or the command
    ; line asks for one.
    Call MultiUser.InstallMode.AllUsers
    ReadRegStr $1 HKLM "${UNINSTALL_KEY}" "UninstallString"
    ${If} $1 == ""
        ReadRegStr $1 HKCU "${UNINSTALL_KEY}" "UninstallString"
        ${If} $1 != ""
            Call MultiUser.InstallMode.CurrentUser
        ${EndIf}
    ${EndIf}
    ${GetParameters} $R1
    ${StrStr} $R2 $R1 "/CurrentUser"
    ${If} $R2 != ""
        Call MultiUser.InstallMode.CurrentUser
    ${EndIf}
    ${StrStr} $R2 $R1 "/AllUsers"
    ${If} $R2 != ""
        Call MultiUser.InstallMode.AllUsers
    ${EndIf}
FunctionEnd

; Marks the copy that was started with raised rights, so a directory that
; stays unwritable cannot send the installer round in circles.
!define ELEVATED_FLAG "/elevated"
; Set only by MaybeElevate after the user confirms the all-users choice. The
; raised copy resumes on the Components page instead of asking the welcome,
; license and installation-mode questions again.
!define RESUME_COMPONENTS_FLAG "/resume-components"

Var Writable
Var CommandLineInstallDir
Var ResumeComponents
; "yes" when installing over an earlier installation of the chosen mode.
Var Upgrade
; Mode and directory DetectUpgrade last looked at.
Var DetectedFor

; Whether this process may create and write in $INSTDIR. A standard account,
; and an administrator who has not been elevated, cannot write under Program
; Files — which is exactly the case that needs a UAC request.
Function ProbeInstallDir
    StrCpy $Writable "no"
    ; A directory the probe itself makes is removed again, so cancelling the
    ; wizard leaves nothing behind under Program Files.
    ${If} ${FileExists} "$INSTDIR\*.*"
        StrCpy $R3 "kept"
    ${Else}
        StrCpy $R3 "probed"
    ${EndIf}
    ClearErrors
    CreateDirectory "$INSTDIR"
    ${If} ${Errors}
        Return
    ${EndIf}
    ClearErrors
    FileOpen $R0 "$INSTDIR\.pwtool-write-test" w
    ${If} ${Errors}
        ${If} $R3 == "probed"
            RMDir "$INSTDIR"
        ${EndIf}
        Return
    ${EndIf}
    FileClose $R0
    Delete "$INSTDIR\.pwtool-write-test"
    StrCpy $Writable "yes"
    ${If} $R3 == "probed"
        ; Only ever removes it while it is still empty; the install section
        ; creates it again for real.
        RMDir "$INSTDIR"
    ${EndIf}
FunctionEnd

; Restarts with a raised token when the all-users directory turns out to be
; read-only, carrying the current command line over so a silent run stays
; silent. Leaves $Writable telling the caller whether installing can go on:
; ShellExecute reports failure — a declined prompt included — with a value
; of 32 or less.
Function MaybeElevate
    StrCpy $Writable "yes"
    ${If} $MultiUser.InstallMode != "AllUsers"
        Return
    ${EndIf}
    ; All-users installation also writes HKLM and common shortcuts; a writable
    ; custom destination alone is not proof of administrator rights.
    UserInfo::GetAccountType
    Pop $0
    ${If} $0 == "Admin"
        Call ProbeInstallDir
        Return
    ${EndIf}
    StrCpy $Writable "no"
    ${GetParameters} $R1
    ${StrStr} $R2 $R1 "${ELEVATED_FLAG}"
    ${If} $R2 != ""
        ; Already the raised copy: the directory is unwritable for some other
        ; reason, and asking again would only loop.
        Return
    ${EndIf}
    StrCpy $R3 "/AllUsers ${ELEVATED_FLAG} ${RESUME_COMPONENTS_FLAG} $R1"
    ; NSIS consumes /D= before .onInit, so restore it as the final argument
    ; for the raised copy. This preserves an explicit custom destination.
    ${If} $CommandLineInstallDir != ""
        StrCpy $R3 "$R3 /D=$CommandLineInstallDir"
    ${EndIf}
    System::Call 'shell32::ShellExecuteW(p 0, t "runas", t "$EXEPATH", \
        t "$R3", t "", i 1) i .r0'
    ${If} $0 > 32
        Quit
    ${EndIf}
FunctionEnd

; Leaving the install mode page: the choice between the two variants is
; known here, and this is the first moment that may need administrator rights.
Function ElevateForAllUsers
    Call MaybeElevate
    ${If} $Writable != "yes"
        MessageBox MB_OK|MB_ICONEXCLAMATION "$(NeedAdmin)"
        ; Back to the page, where «Только для меня» still works.
        Abort
    ${EndIf}
FunctionEnd

; Started through Explorer so that an elevated installer does not hand its
; administrator rights to the program (autotype into normal windows would
; behave differently, and the settings would belong to the wrong account).
Function RunProgram
    Exec '"$WINDIR\explorer.exe" "$INSTDIR\${EXE}"'
FunctionEnd

Function InstallModeChanged
    ${If} $CommandLineInstallDir != ""
        StrCpy $INSTDIR $CommandLineInstallDir
        Return
    ${EndIf}
    ; Keep a directory chosen during an earlier installation; only replace the
    ; plain defaults of MultiUser.nsh with the ones this program wants.
    ${If} $MultiUser.InstallMode == "AllUsers"
        ${If} $INSTDIR == "$PROGRAMFILES64\${SHORT_NAME}"
        ${OrIf} $INSTDIR == "$PROGRAMFILES32\${SHORT_NAME}"
            StrCpy $INSTDIR "$PROGRAMFILES64\${SHORT_NAME}"
        ${EndIf}
    ${Else}
        ${If} $INSTDIR == "$LOCALAPPDATA\${SHORT_NAME}"
            StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${SHORT_NAME}"
        ${EndIf}
    ${EndIf}
FunctionEnd

; The all-users choice is made on the page immediately before Components.
; After UAC starts the elevated copy, the preceding pages are already
; complete. Aborting their pre-callbacks moves the wizard directly forward.
Function ResumeBeforeComponents
    ${If} $ResumeComponents == "yes"
        Abort
    ${EndIf}
FunctionEnd

Function .onInit
    ${IfNot} ${RunningX64}
        MessageBox MB_OK|MB_ICONSTOP "$(Need64)" /SD IDOK
        Abort
    ${EndIf}
    SetRegView 64
    ; NSIS has already parsed the special, last /D= argument (including
    ; spaces) and removed it from $CMDLINE: keep that result.
    StrCpy $CommandLineInstallDir $INSTDIR
    StrCpy $ResumeComponents "no"
    ${GetParameters} $R1
    ${StrStr} $R2 $R1 "${ELEVATED_FLAG}"
    ${If} $R2 != ""
        ${StrStr} $R2 $R1 "${RESUME_COMPONENTS_FLAG}"
        ${If} $R2 != ""
            StrCpy $ResumeComponents "yes"
        ${EndIf}
    ${EndIf}
    !insertmacro MUI_LANGDLL_DISPLAY
    !insertmacro MULTIUSER_INIT
    Call RestoreAdminPrivileges
    ; Respect the documented NSIS /D= override after MultiUser chooses defaults.
    ${If} $CommandLineInstallDir != ""
        StrCpy $INSTDIR $CommandLineInstallDir
    ${EndIf}
    ; A silent run shows no pages: the components are set here.
    Call DetectUpgrade
    ; A silent run shows no pages, so the all-users check has to happen here.
    ${If} ${Silent}
        Call MaybeElevate
        ${If} $Writable != "yes"
            SetErrorLevel 740 ; ERROR_ELEVATION_REQUIRED
            Quit
        ${EndIf}
    ${EndIf}
FunctionEnd

Function un.onInit
    SetRegView 64
    !insertmacro MUI_UNGETLANGUAGE
    !insertmacro MULTIUSER_UNINIT
    ; Removing an all-users installation needs the same rights its files do.
    ${If} $MultiUser.Privileges != "Admin"
    ${AndIf} $MultiUser.Privileges != "Power"
        UserInfo::GetOriginalAccountType
        Pop $0
        ${If} $0 == "Admin"
            StrCpy $MultiUser.Privileges "Admin"
        ${EndIf}
    ${EndIf}
    ${If} $MultiUser.InstallMode == "AllUsers"
        ${un.GetParameters} $R1
        ${UnStrStr} $R2 $R1 "${ELEVATED_FLAG}"
        ${If} $R2 == ""
            UserInfo::GetAccountType
            Pop $0
            ${If} $0 != "Admin"
                System::Call 'shell32::ShellExecuteW(p 0, t "runas", \
                    t "$INSTDIR\uninstall.exe", t "/AllUsers ${ELEVATED_FLAG} $R1", \
                    t "", i 1) i .r0'
                ${If} $0 > 32
                    Quit
                ${EndIf}
                MessageBox MB_OK|MB_ICONEXCLAMATION "$(NeedAdminUninstall)" /SD IDOK
                Quit
            ${EndIf}
        ${EndIf}
    ${EndIf}
FunctionEnd

; --------------------------------------------------------------- sections

Section "!${NAME}" SecMain
    SectionIn RO
    ${GetRoot} "$INSTDIR" $0
    ${If} $0 == ""
    ${OrIf} $INSTDIR == "$0"
    ${OrIf} $INSTDIR == "$0\"
        SetErrorLevel 87
        Abort "Choose a program subdirectory, not a drive root."
    ${EndIf}
    Call WaitForProgram
    SetOutPath "$INSTDIR"

    File "/oname=${EXE}" "${SOURCE_EXE}"
    File "/oname=LICENSE.txt" "${ROOT}/LICENSE"

    CreateDirectory "$SMPROGRAMS\${NAME}"
    CreateShortcut "$SMPROGRAMS\${NAME}\${NAME}.lnk" "$INSTDIR\${EXE}"

    WriteRegStr SHCTX "Software\${SHORT_NAME}" "InstallDir" "$INSTDIR"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/coinman-dev/PwTool"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe" /$MultiUser.InstallMode'
    WriteRegStr SHCTX "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /$MultiUser.InstallMode /S'
    WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "NoRepair" 1
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD SHCTX "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

Section /o "$(NameDesktop)" SecDesktop
    CreateShortcut "$DESKTOP\${NAME}.lnk" "$INSTDIR\${EXE}"
SectionEnd

; Off by default: a password generator does not need to run all the time.
Section /o "$(NameAutostart)" SecAutostart
    ; The same value the program writes itself, so its «Запускать при старте
    ; системы» checkbox shows the real state after the first run.
    WriteRegStr HKCU "${RUN_KEY}" "${RUN_VALUE}" '"$INSTDIR\${EXE}" --silent'
SectionEnd

; An update over an earlier installation keeps what the user has chosen
; since: autostart is set in the program itself, so the component is hidden
; and the registration left alone; an existing desktop shortcut is kept.
Function DetectUpgrade
    ${If} $DetectedFor == "$MultiUser.InstallMode|$INSTDIR"
        Return
    ${EndIf}
    StrCpy $DetectedFor "$MultiUser.InstallMode|$INSTDIR"
    StrCpy $Upgrade "no"
    ReadRegStr $0 SHCTX "${UNINSTALL_KEY}" "InstallLocation"
    ${If} $0 != ""
    ${AndIf} ${FileExists} "$0\${EXE}"
        StrCpy $Upgrade "yes"
    ${EndIf}
    ${If} $Upgrade == "yes"
        SectionSetFlags ${SecAutostart} 0
        SectionSetText ${SecAutostart} ""
        ${If} ${FileExists} "$DESKTOP\${NAME}.lnk"
            SectionSetFlags ${SecDesktop} ${SF_SELECTED}
        ${EndIf}
    ${Else}
        SectionSetText ${SecAutostart} "$(NameAutostart)"
    ${EndIf}
FunctionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
    !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "$(DescMain)"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "$(DescDesktop)"
    !insertmacro MUI_DESCRIPTION_TEXT ${SecAutostart} "$(DescAutostart)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ------------------------------------------------------------- uninstall

Section "Uninstall"
    ${un.GetRoot} "$INSTDIR" $0
    ${If} $0 == ""
    ${OrIf} $INSTDIR == "$0"
    ${OrIf} $INSTDIR == "$0\"
        SetErrorLevel 87
        Abort "Invalid installation directory."
    ${EndIf}
    Call un.WaitForProgram

    ; Leave another installation's autostart alone.
    ReadRegStr $0 HKCU "${RUN_KEY}" "${RUN_VALUE}"
    ${If} $0 == '"$INSTDIR\${EXE}" --silent'
        DeleteRegValue HKCU "${RUN_KEY}" "${RUN_VALUE}"
    ${EndIf}

    Delete "$SMPROGRAMS\${NAME}\${NAME}.lnk"
    RMDir "$SMPROGRAMS\${NAME}"
    Delete "$DESKTOP\${NAME}.lnk"

    Delete "$INSTDIR\${EXE}"
    Delete "$INSTDIR\LICENSE.txt"

    DeleteRegKey SHCTX "${UNINSTALL_KEY}"
    DeleteRegKey SHCTX "Software\${SHORT_NAME}"

    Call un.AskAboutUserData
    ; Last, so a failure above still leaves a way to remove the program.
    Delete "$INSTDIR\uninstall.exe"
    ; Only if empty: a PwTool.ini put there by hand (portable mode) and the
    ; files of that mode stay.
    RMDir "$INSTDIR"
SectionEnd

; The settings are worth keeping across a reinstall, so removing them is a
; separate answer; a silent uninstall keeps them. Only the settings of the
; account running the uninstaller are known (each user has their own
; %APPDATA%), and only the program's own files are deleted.
Function un.AskAboutUserData
    ; the embedded data the program unpacked (translations, manuals)
    RMDir /r "$TEMP\${SHORT_NAME}"
    ${If} ${Silent}
        Return
    ${EndIf}
    SetShellVarContext current
    StrCpy $0 "$APPDATA\${SHORT_NAME}"
    ${If} $MultiUser.InstallMode == "AllUsers"
        SetShellVarContext all
    ${EndIf}
    ${IfNot} ${FileExists} "$0\${INI_FILE}"
    ${AndIfNot} ${FileExists} "$0\${SEED_FILE}"
        Return
    ${EndIf}
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(AskRemoveData)" /SD IDNO IDYES remove
    Return
    remove:
    Delete "$0\${INI_FILE}"
    Delete "$0\${SEED_FILE}"
    ClearErrors
    RMDir "$0"
    ${If} ${Errors}
        MessageBox MB_OK|MB_ICONINFORMATION "$(KeptFiles)$\r$\n$0"
    ${EndIf}
FunctionEnd
