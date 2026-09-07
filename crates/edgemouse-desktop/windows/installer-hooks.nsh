!include TextFunc.nsh

; Tauri's default MUI macro silently reuses the language stored by old releases.
; Manual installs always offer both languages, starting with Simplified Chinese.
; App updates pass /LANG=1033 or /LANG=2052 and never show a blocking dialog.
!macroundef MUI_LANGDLL_DISPLAY
!macro MUI_LANGDLL_DISPLAY
  Push $R9
  StrCpy $LANGUAGE 2052
  ClearErrors
  ${GetOptions} $CMDLINE "/LANG=" $R9
  ${IfNot} ${Errors}
    ${If} $R9 == 1033
      StrCpy $LANGUAGE 1033
    ${EndIf}
  ${Else}
    ${Unless} ${Silent}
      ${If} $PassiveMode != 1
        LangDLL::LangDialog "安装语言 / Setup language" "请选择安装语言 / Please select a language" A ${MUI_LANGDLL_LANGUAGES} ""
        Pop $LANGUAGE
        ${If} $LANGUAGE == "cancel"
          Abort
        ${EndIf}
      ${EndIf}
    ${EndUnless}
  ${EndIf}
  ClearErrors
  Pop $R9
!macroend

; This also protects upgrades launched by old clients that do not stop their
; sidecar before starting the installer. Never forcibly kill arbitrary processes.
!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\edgemouse.exe" 0 edgemouse_ready
  nsExec::ExecToStack /TIMEOUT=10000 '"$INSTDIR\edgemouse.exe" stop'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_ICONSTOP|MB_OK "无法安全停止 EdgeMouse 后台服务，请退出后重试。 / Could not stop the background service safely. Please exit EdgeMouse and retry." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  StrCpy $2 0
  edgemouse_wait_for_exit:
    ; Append mode does not truncate or write the file. An open image cannot be
    ; opened for writing, so do not replace either component until this succeeds.
    ClearErrors
    FileOpen $0 "$INSTDIR\edgemouse.exe" a
    ${IfNot} ${Errors}
      FileClose $0
      Goto edgemouse_ready
    ${EndIf}
    IntOp $2 $2 + 1
    ${If} $2 < 20
      Sleep 250
      Goto edgemouse_wait_for_exit
    ${EndIf}
    MessageBox MB_ICONSTOP|MB_OK "后台程序仍被占用，安装已停止，以免版本不一致。请退出 EdgeMouse 后重试。 / The background executable is still in use. Installation stopped to prevent a mixed-version install." /SD IDOK
    SetErrorLevel 1
    Abort
  edgemouse_ready:
  ClearErrors
!macroend

!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToStack /TIMEOUT=10000 '"$INSTDIR\edgemouse.exe" version'
  Pop $0
  Pop $1
  ${TrimNewLines} $1 $1
  ${If} $0 != 0
  ${OrIf} $1 != "edgemouse ${VERSION}"
    MessageBox MB_ICONSTOP|MB_OK "后台程序版本核验失败，请重新运行当前安装包修复。 / Background version verification failed. Please run this installer again to repair the installation." /SD IDOK
    SetErrorLevel 1
    Abort
  ${EndIf}
  ; Refresh old opaque shell icons without restarting Explorer or deleting caches.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
