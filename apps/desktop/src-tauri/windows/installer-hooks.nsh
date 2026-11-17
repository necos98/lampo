; Hooks of Tauri's NSIS installer (bundle.windows.nsis.installerHooks in tauri.bundle.conf.json).
; Tauri includes this file at the top of its installer.nsi, after its own !include lines and
; before its !define lines: functions here cannot use ${PRODUCTNAME} & co. (expanded at parse
; time), macros can.
;
; --- Install folder: %LOCALAPPDATA%\Programs\Lampo, never Lampo's data root ---
;
; With installMode "currentUser" Tauri installs into $LOCALAPPDATA\${PRODUCTNAME} =
; %LOCALAPPDATA%\Lampo, which is also Lampo's data root (packages, MySQL data, logs, shims:
; APP_DIR_NAME in crates/lampo-infra-store/src/paths.rs, hard-coded below for that reason). The
; app goes to %LOCALAPPDATA%\Programs\Lampo instead (per user, no admin, like VS Code), so
; deleting either folder never touches the other.
;
; Tauri's .onInit sets $INSTDIR to its default and then to the previous install location
; (registry, Software\<manufacturer>\<product>): updates and reinstalls stay where the app is.
; LampoAvoidDataRoot only redirects $INSTDIR when it is the data root, at three points:
;   * .onGUIInit (MUI_CUSTOMFUNCTION_GUIINIT): interactive and passive (/P, the updater) runs,
;     so the directory page shows the real folder;
;   * NSIS_HOOK_PREINSTALL: silent runs (/S), which have no GUI init;
;   * .onVerifyInstDir: the directory page refuses the data root if typed in by hand.
; The uninstaller, the registry keys and the shortcuts use $INSTDIR, so they follow.

!define LAMPO_DATA_ROOT_NAME "Lampo"
!define LAMPO_INSTALL_PARENT "Programs"

Function LampoAvoidDataRoot
  ${If} $INSTDIR == "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}"
  ${OrIf} $INSTDIR == "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\"
    StrCpy $INSTDIR "$LOCALAPPDATA\${LAMPO_INSTALL_PARENT}\${LAMPO_DATA_ROOT_NAME}"
  ${EndIf}
FunctionEnd

!define MUI_CUSTOMFUNCTION_GUIINIT LampoGuiInit
Function LampoGuiInit
  Call LampoAvoidDataRoot
FunctionEnd

Function .onVerifyInstDir
  ${If} $INSTDIR == "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}"
  ${OrIf} $INSTDIR == "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\"
    Abort ; disables "Install": this folder holds Lampo's data
  ${EndIf}
FunctionEnd

; --- Helper processes ---
;
; The installer closes lampo-desktop.exe by itself, but not the sidecars that ship next to it:
; a `lampo.exe` CLI still running from $INSTDIR (e.g. `lampo logs --follow` in a terminal) or a
; `lampo-hosts-helper.exe` would keep their files locked and the update would fail to replace
; them. Both are short-lived helpers, so they are terminated before the files are copied.
;
; Only processes whose executable lies under $INSTDIR are closed, never by image name: the
; lampo.exe copy in Lampo's shim folder (a `lampo exec` wrapping a dev server, a Claude Code hook)
; and the shims themselves do not lock $INSTDIR, and the app refreshes them at the next start.
; Two processes under $INSTDIR are spared: lampo-desktop.exe, which Tauri's CheckIfAppIsRunning
; closes right after this hook (asking the user, so the app stops its services cleanly), and the
; running installer itself. The folder and the exceptions reach PowerShell as environment
; variables, never inside the command text, and are compared as plain strings (no wildcards).
;
; The installer is a 32-bit process, so $SYSDIR's PowerShell is the 32-bit one (WOW64 file system
; redirection), whose Get-Process has no Path for 64-bit processes such as lampo.exe: on 64-bit
; Windows the native PowerShell is reached through Sysnative, which only 32-bit processes see.
;
; The hosts helper runs elevated (hosts file, Defender exclusion): a per-user installer cannot read
; its path nor stop it, which is harmless (it exits on its own within seconds). Errors and the exit
; code are ignored.

!define LAMPO_POWERSHELL_SUBPATH "WindowsPowerShell\v1.0\powershell.exe"
!define LAMPO_ENV_INSTALL_DIR "LAMPO_INSTALLER_INSTALL_DIR"
!define LAMPO_ENV_KEEP_APP "LAMPO_INSTALLER_KEEP_APP"
!define LAMPO_ENV_KEEP_SELF "LAMPO_INSTALLER_KEEP_SELF"

!macro LampoSetEnv NAME VALUE
  System::Call 'kernel32::SetEnvironmentVariable(t "${NAME}", t "${VALUE}") i'
!macroend

!macro LampoClearEnv NAME
  System::Call 'kernel32::SetEnvironmentVariable(t "${NAME}", p 0) i'
!macroend

!macro LampoCloseProcessesUnderInstDir
  !insertmacro LampoSetEnv ${LAMPO_ENV_INSTALL_DIR} "$INSTDIR"
  !insertmacro LampoSetEnv ${LAMPO_ENV_KEEP_APP} "$INSTDIR\${MAINBINARYNAME}.exe"
  !insertmacro LampoSetEnv ${LAMPO_ENV_KEEP_SELF} "$EXEPATH"
  Push $0
  StrCpy $0 "$SYSDIR\${LAMPO_POWERSHELL_SUBPATH}"
  ${If} ${FileExists} "$WINDIR\Sysnative\${LAMPO_POWERSHELL_SUBPATH}"
    StrCpy $0 "$WINDIR\Sysnative\${LAMPO_POWERSHELL_SUBPATH}"
  ${EndIf}
  nsExec::Exec '"$0" -NoLogo -NoProfile -NonInteractive -Command "$$dir = $$env:${LAMPO_ENV_INSTALL_DIR}.TrimEnd([char]92) + [char]92; Get-Process | Where-Object { $$_.Path -and $$_.Path.StartsWith($$dir, [StringComparison]::OrdinalIgnoreCase) -and $$_.Path -ne $$env:${LAMPO_ENV_KEEP_APP} -and $$_.Path -ne $$env:${LAMPO_ENV_KEEP_SELF} } | Stop-Process -Force -ErrorAction SilentlyContinue"'
  Pop $0 ; exit code of PowerShell
  Pop $0 ; restore $0
  !insertmacro LampoClearEnv ${LAMPO_ENV_INSTALL_DIR}
  !insertmacro LampoClearEnv ${LAMPO_ENV_KEEP_APP}
  !insertmacro LampoClearEnv ${LAMPO_ENV_KEEP_SELF}
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Silent installs skip .onGUIInit: redirect here too (Tauri's SetOutPath already ran).
  Call LampoAvoidDataRoot
  SetOutPath $INSTDIR
  !insertmacro LampoCloseProcessesUnderInstDir
!macroend

; --- Final uninstall vs. a step of an update or reinstall ---
;
; The uninstaller also runs as one step of putting the app back, and then nothing outside
; $INSTDIR may go:
;   * /UPDATE: an update (the updater, or a setup run in update mode);
;   * in place (`_?=$INSTDIR`, so $EXEDIR is $INSTDIR): the reinstall page of a setup.exe run by
;     hand (newer, same or older version), which runs the previous uninstaller without /UPDATE
;     and installs right after.
; A real uninstall (Settings > Apps, uninstall.exe, /S) first copies the uninstaller to %TEMP% and
; runs it from there, so $EXEDIR differs from $INSTDIR. LampoDetectFinalUninstall sets
; $LampoFinalUninstall once, at the start of the uninstall section.

Var LampoFinalUninstall

!macro LampoDetectFinalUninstall
  StrCpy $LampoFinalUninstall 1
  ${If} $UpdateMode = 1
  ${OrIf} $EXEDIR == $INSTDIR
    StrCpy $LampoFinalUninstall 0
  ${EndIf}
!macroend

; --- Command-line shims ---
;
; `lampo shims install` (Settings > Strumenti da riga di comando) copies the shims and the CLI
; into Lampo's shim folder and adds it to the user PATH. A final uninstall undoes both: the PATH
; entry goes first (other entries and the value type are kept), then the copies (the shim index
; and the rest of the data root stay). It runs after Tauri's CheckIfAppIsRunning, so an uninstall
; cancelled at the "app is running" prompt leaves the shims alone; by then $INSTDIR is gone, so
; the pre-uninstall step keeps a copy of the CLI in $PLUGINSDIR (deleted when the uninstaller
; exits). The exit code is only reported: a shim still running keeps its file, but the PATH is
; clean anyway.

!define LAMPO_CLI_EXE "lampo.exe"

!macro LampoKeepCliForShimUninstall
  ${If} ${FileExists} "$INSTDIR\${LAMPO_CLI_EXE}"
    InitPluginsDir
    CopyFiles /SILENT "$INSTDIR\${LAMPO_CLI_EXE}" "$PLUGINSDIR"
  ${EndIf}
!macroend

!macro LampoUninstallShims
  ${If} ${FileExists} "$PLUGINSDIR\${LAMPO_CLI_EXE}"
    Push $0
    nsExec::Exec '"$PLUGINSDIR\${LAMPO_CLI_EXE}" shims uninstall'
    Pop $0
    ${If} $0 != 0
      DetailPrint "lampo shims uninstall: exit code $0"
    ${EndIf}
    Pop $0
  ${EndIf}
!macroend

; --- Claude Code plugin ---
;
; The app registers its Claude Code plugin (user scope) only when the user asks for it (Settings >
; Integrazioni > Claude Code); it then writes a marker in the generated plugin folder. A final
; uninstall unregisters it with the official CLI and deletes that generated folder; nothing else
; in the data root is touched. The exit codes are ignored: Claude Code may have been removed
; already. `cmd /c` resolves the npm shim (claude.cmd) as well as the native claude.exe.

!define LAMPO_CLAUDE_PLUGIN_DIR "agent-plugins\claude-code"

!macro LampoUnregisterClaudePlugin
  ${If} ${FileExists} "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\${LAMPO_CLAUDE_PLUGIN_DIR}\.installed-version"
    Push $0
    nsExec::Exec '"$SYSDIR\cmd.exe" /d /c claude plugin uninstall lampo@lampo --scope user'
    Pop $0
    nsExec::Exec '"$SYSDIR\cmd.exe" /d /c claude plugin marketplace remove lampo'
    Pop $0
    Pop $0
    RMDir /r "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\${LAMPO_CLAUDE_PLUGIN_DIR}"
  ${EndIf}
!macroend

; --- Uninstall hooks ---

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro LampoDetectFinalUninstall
  ${If} $LampoFinalUninstall = 1
    !insertmacro LampoKeepCliForShimUninstall
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $LampoFinalUninstall = 1
    !insertmacro LampoUninstallShims
    !insertmacro LampoUnregisterClaudePlugin
  ${EndIf}
!macroend
