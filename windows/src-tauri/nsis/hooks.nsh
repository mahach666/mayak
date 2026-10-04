; Uninstall hooks for the NSIS installer.
;
; The app stages mayak-hook.exe into %LOCALAPPDATA%\Mayak\bin at launch, so the
; installer never recorded it and the default uninstaller leaves it behind. The
; inbox and the log live in the same place and are ours too.
;
; Tool configs are deliberately NOT touched here: ~/.codex/hooks.json and
; ~/.claude/settings.json belong to the user, they may hold hooks from other
; tools, and rewriting somebody's config from an uninstaller with no diff and no
; consent is exactly what the rest of this app goes out of its way not to do.
; When a Codex hooks.json exists, the uninstaller only reminds the user to remove
; the integration from Mayak's settings first (spec section 8). OK continues,
; Cancel stops the uninstall before anything is deleted (this hook runs before
; the template's CheckIfAppIsRunning, so Mayak may still be open to do it). An
; update (/UPDATE) or a passive run (/P, which a newer setup started with /P
; passes on when it removes this version) skips the reminder; $UpdateMode and
; $PassiveMode are the template's own globals, set in un.onInit. ASCII only:
; NSIS reads a .nsh without BOM in the ANSI code page.

!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
  ${AndIf} $PassiveMode <> 1
    Push $0
    ReadEnvStr $0 CODEX_HOME
    StrCmp $0 "" 0 +2
      StrCpy $0 "$PROFILE\.codex"
    IfFileExists "$0\hooks.json" 0 mayak_codex_hint_done
      MessageBox MB_OKCANCEL|MB_ICONINFORMATION "If you installed Mayak's Codex integration, remove it first: Mayak > Settings > Codex > Remove. The uninstaller does not edit $0\hooks.json.$\r$\nOK continues, Cancel stops the uninstall." /SD IDOK IDOK mayak_codex_hint_done
      Pop $0
      Abort
    mayak_codex_hint_done:
    Pop $0
  ${EndIf}
  RMDir /r "$LOCALAPPDATA\Mayak\bin"
  RMDir /r "$LOCALAPPDATA\Mayak\inbox"
  Delete "$LOCALAPPDATA\Mayak\mayak.log"
!macroend
