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
; a `lampo.exe` CLI still running (e.g. `lampo logs --follow` in a terminal) or a
; `lampo-hosts-helper.exe` would keep their files locked and the update would fail to replace
; them. Both are short-lived helpers, so they are terminated before the files are copied.
; lampo-shim.exe copies (php.exe, composer.exe...) and the lampo.exe copy live in Lampo's shim
; folder, not in $INSTDIR: the app refreshes them at the next start.
;
; The hosts helper runs elevated: from a per-user installer taskkill may be denied, which is
; harmless (it exits on its own within seconds). The exit codes are ignored ("not running" = 128).

!macro LampoKillProcess IMAGE
  Push $0
  nsExec::Exec 'taskkill /F /IM ${IMAGE}'
  Pop $0 ; exit code of taskkill
  Pop $0 ; restore $0
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; Silent installs skip .onGUIInit: redirect here too (Tauri's SetOutPath already ran).
  Call LampoAvoidDataRoot
  SetOutPath $INSTDIR
  !insertmacro LampoKillProcess "lampo.exe"
  !insertmacro LampoKillProcess "lampo-hosts-helper.exe"
!macroend

; --- Claude Code plugin ---
;
; The app registers its Claude Code plugin (user scope) only when the user asks for it (Settings >
; Integrazioni > Claude Code); it then writes a marker in the generated plugin folder. A real
; uninstall (not an update, which runs the uninstaller with /UPDATE) unregisters it with the
; official CLI and deletes that generated folder; nothing else in the data root is touched. The
; exit codes are ignored: Claude Code may have been removed already. `cmd /c` resolves the npm
; shim (claude.cmd) as well as the native claude.exe.

!define LAMPO_CLAUDE_PLUGIN_DIR "agent-plugins\claude-code"

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} ${FileExists} "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\${LAMPO_CLAUDE_PLUGIN_DIR}\.installed-version"
    Push $0
    nsExec::Exec '"$SYSDIR\cmd.exe" /d /c claude plugin uninstall lampo@lampo --scope user'
    Pop $0
    nsExec::Exec '"$SYSDIR\cmd.exe" /d /c claude plugin marketplace remove lampo'
    Pop $0
    Pop $0
    RMDir /r "$LOCALAPPDATA\${LAMPO_DATA_ROOT_NAME}\${LAMPO_CLAUDE_PLUGIN_DIR}"
  ${EndIf}
!macroend
