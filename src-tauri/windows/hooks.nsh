LangString CapstanOpenHere 1033 "Open Capstan here"
LangString CapstanOpenHere 2052 "在此处打开 Capstan"
LangString CapstanSshProtocol 1033 "URL:SSH Protocol"
LangString CapstanSshProtocol 2052 "URL:SSH 协议"
LangString CapstanUpdateVerificationUnavailable 1033 "Update verification could not run. Keep the recovery backup and try again."
LangString CapstanUpdateVerificationUnavailable 2052 "无法执行更新校验，请保留恢复备份后重试。"
LangString CapstanUpdatePayloadInvalid 1033 "Update files could not be verified. The previous installation will be restored; keep the recovery backup."
LangString CapstanUpdatePayloadInvalid 2052 "无法验证更新文件，将恢复原安装，请保留恢复备份。"

; The stock Tauri NSIS installer overwrites files in place. Keep a sibling backup
; before the first write, and retain it if rollback itself cannot complete.
Var CapstanUpdateBackup
Var CapstanUpdateReady
Var CapstanUpdateCommitted
Var CapstanPreviousVersion
Var CapstanPayloadCheckReady
Var CapstanRollbackFailed
!define CAPSTAN_UPDATE_HOOK_DIR "${__FILEDIR__}"

Function CapstanRollbackUpdate
  ${If} $CapstanUpdateReady == 1
  ${AndIf} $CapstanUpdateCommitted != 1
    ClearErrors
    StrCpy $CapstanRollbackFailed 0
    ${If} $CapstanPayloadCheckReady == 1
      ; Use an independent embedded copy: a missing verification script must not
      ; prevent cleanup of payload additions. Never remove untracked user paths.
      nsExec::ExecToStack /TIMEOUT=60000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\capstan-rollback.ps1" "$INSTDIR\." -Mode Rollback -InventoryPath "$PLUGINSDIR\capstan-additions.lst" -BackupDirectory "$CapstanUpdateBackup\."'
      Pop $0
      Pop $1
      ${If} $0 != "0"
        StrCpy $CapstanRollbackFailed 1
      ${EndIf}
    ${Else}
      ; Minimal failure harnesses have no payload script. Production restores
      ; each file through the script, so one unchanged locked file cannot stop
      ; the restoration of later files in the backup.
      CopyFiles /SILENT "$CapstanUpdateBackup\*.*" "$INSTDIR"
      ${If} ${Errors}
        StrCpy $CapstanRollbackFailed 1
      ${EndIf}
    ${EndIf}
    ${If} $CapstanRollbackFailed == 0
      !ifndef CAPSTAN_UPDATE_HARNESS
        ${If} $CapstanPreviousVersion != ""
          WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Capstan" "DisplayVersion" "$CapstanPreviousVersion"
        ${EndIf}
      !endif
      StrCpy $CapstanUpdateReady 0
    ${EndIf}
    ; Never delete the recovery copy on failure, including a restore failure.
  ${EndIf}
FunctionEnd

Function .onInstFailed
  Call CapstanRollbackUpdate
FunctionEnd

Function .onGUIEnd
  Call CapstanRollbackUpdate
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  ; Generate the complete payload inventory from the same resource mapping as
  ; Tauri, including the just-built main executable. The checker is installer data,
  ; not a mutable sidecar beside the downloaded installer.
  !ifndef CAPSTAN_UPDATE_PAYLOAD_CHECK
    !ifndef CAPSTAN_UPDATE_HARNESS
      !tempfile CAPSTAN_UPDATE_PAYLOAD_CHECK
      !define CAPSTAN_GENERATED_PAYLOAD_CHECK
      !system 'python "${CAPSTAN_UPDATE_HOOK_DIR}\..\..\scripts\release.py" nsis-payload-check "${MAINBINARYSRCPATH}" "${CAPSTAN_UPDATE_HOOK_DIR}\..\tauri.conf.json" "${CAPSTAN_UPDATE_PAYLOAD_CHECK}"' = 0
    !endif
  !endif
  !ifdef CAPSTAN_UPDATE_PAYLOAD_CHECK
    InitPluginsDir
    ClearErrors
    File "/oname=$PLUGINSDIR\capstan-payload-check.ps1" "${CAPSTAN_UPDATE_PAYLOAD_CHECK}"
    File "/oname=$PLUGINSDIR\capstan-rollback.ps1" "${CAPSTAN_UPDATE_PAYLOAD_CHECK}"
    ${If} ${Errors}
      SetErrorLevel 1
      Abort "$(CapstanUpdateVerificationUnavailable)"
    ${EndIf}
    StrCpy $CapstanPayloadCheckReady 1
    !ifdef CAPSTAN_GENERATED_PAYLOAD_CHECK
      !delfile "${CAPSTAN_UPDATE_PAYLOAD_CHECK}"
    !endif
  !endif
  ${If} $UpdateMode == 1
  ${AndIf} ${FileExists} "$INSTDIR\capstan-app.exe"
    StrCpy $CapstanUpdateBackup "$INSTDIR.capstan-update-backup"
    ; A prior interrupted update needs explicit manual recovery; do not overwrite it.
    ${If} ${FileExists} "$CapstanUpdateBackup"
      Abort "update_recovery_pending"
    ${EndIf}
    !ifndef CAPSTAN_UPDATE_HARNESS
      ReadRegStr $CapstanPreviousVersion HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\Capstan" "DisplayVersion"
    !endif
    CreateDirectory "$CapstanUpdateBackup"
    ClearErrors
    CopyFiles /SILENT "$INSTDIR\*.*" "$CapstanUpdateBackup"
    ${If} ${Errors}
      Abort "update_backup_failed"
    ${EndIf}
    ${IfNot} ${FileExists} "$CapstanUpdateBackup\capstan-app.exe"
      Abort "update_backup_failed"
    ${EndIf}
    ${If} $CapstanPayloadCheckReady == 1
      ; Capture absence before the first installed payload write. Existing empty
      ; directories and user files are deliberately absent from this inventory.
      nsExec::ExecToStack /TIMEOUT=60000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\capstan-rollback.ps1" "$INSTDIR\." -Mode Prepare -InventoryPath "$PLUGINSDIR\capstan-additions.lst"'
      Pop $0
      Pop $1
      ${If} $0 != "0"
        SetErrorLevel 1
        Abort "$(CapstanUpdateVerificationUnavailable)"
      ${EndIf}
    ${EndIf}
    StrCpy $CapstanUpdateReady 1
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\Directory\shell\Capstan" "" "$(CapstanOpenHere)"
  WriteRegStr HKCU "Software\Classes\Directory\shell\Capstan" "CapstanOwner" "$INSTDIR\capstan-app.exe"
  WriteRegStr HKCU "Software\Classes\Directory\shell\Capstan" "Icon" "$INSTDIR\capstan-app.exe"
  ; A forward slash keeps a trailing backslash in %V from escaping the closing quote.
  WriteRegStr HKCU "Software\Classes\Directory\shell\Capstan\command" "" '"$INSTDIR\capstan-app.exe" --local --cwd "%V/"'
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Capstan" "" "$(CapstanOpenHere)"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Capstan" "CapstanOwner" "$INSTDIR\capstan-app.exe"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Capstan" "Icon" "$INSTDIR\capstan-app.exe"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Capstan\command" "" '"$INSTDIR\capstan-app.exe" --local --cwd "%V/"'

  WriteRegStr HKCU "Software\RegisteredApplications" "Capstan" "Software\Capstan\Capabilities"
  WriteRegStr HKCU "Software\Capstan\Capabilities" "ApplicationName" "Capstan"
  WriteRegStr HKCU "Software\Capstan\Capabilities" "CapstanOwner" "$INSTDIR\capstan-app.exe"
  WriteRegStr HKCU "Software\Capstan\Capabilities\URLAssociations" "ssh" "Capstan.ssh"
  WriteRegStr HKCU "Software\Classes\Capstan.ssh" "" "$(CapstanSshProtocol)"
  WriteRegStr HKCU "Software\Classes\Capstan.ssh" "CapstanOwner" "$INSTDIR\capstan-app.exe"
  WriteRegStr HKCU "Software\Classes\Capstan.ssh" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\Capstan.ssh\shell\open\command" "" '"$INSTDIR\capstan-app.exe" "%1"'
  !insertmacro CapstanCommitUpdate
!macroend

!macro CapstanCommitUpdate
  ; File can report a refused write without aborting. Reject that failure even in
  ; minimal harnesses; production also checks actual bytes after GetSize/shortcuts
  ; and other template operations may have reset the extraction error flag.
  ${If} ${Errors}
    SetErrorLevel 1
    Abort "$(CapstanUpdatePayloadInvalid)"
  ${EndIf}
  ${If} $CapstanPayloadCheckReady != 1
    SetErrorLevel 1
    Abort "$(CapstanUpdateVerificationUnavailable)"
  ${Else}
    nsExec::ExecToStack /TIMEOUT=60000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\capstan-payload-check.ps1" "$INSTDIR\."'
    Pop $0
    Pop $1
    ${If} $0 != "0"
      SetErrorLevel 1
      Abort "$(CapstanUpdatePayloadInvalid)"
    ${EndIf}
  ${EndIf}
  ; WriteUninstaller has no build-time payload digest. The template's earlier
  ; write error may have been cleared by GetSize, so generate it again here and
  ; check the result immediately, before any instruction can reset that flag.
  ClearErrors
  WriteUninstaller "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    SetErrorLevel 1
    Abort "$(CapstanUpdatePayloadInvalid)"
  ${EndIf}
  StrCpy $CapstanUpdateCommitted 1
  ${If} $CapstanUpdateReady == 1
    RMDir /r "$CapstanUpdateBackup"
    StrCpy $CapstanUpdateReady 0
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Push $0
  ReadRegStr $0 HKCU "Software\Classes\Directory\shell\Capstan" "CapstanOwner"
  ${If} $0 == "$INSTDIR\capstan-app.exe"
    DeleteRegKey HKCU "Software\Classes\Directory\shell\Capstan"
  ${EndIf}
  ReadRegStr $0 HKCU "Software\Classes\Directory\Background\shell\Capstan" "CapstanOwner"
  ${If} $0 == "$INSTDIR\capstan-app.exe"
    DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\Capstan"
  ${EndIf}
  ReadRegStr $0 HKCU "Software\Capstan\Capabilities" "CapstanOwner"
  ${If} $0 == "$INSTDIR\capstan-app.exe"
    ReadRegStr $0 HKCU "Software\RegisteredApplications" "Capstan"
    ${If} $0 == "Software\Capstan\Capabilities"
      DeleteRegValue HKCU "Software\RegisteredApplications" "Capstan"
    ${EndIf}
    DeleteRegKey HKCU "Software\Capstan\Capabilities"
    DeleteRegKey /ifempty HKCU "Software\Capstan"
  ${EndIf}
  ReadRegStr $0 HKCU "Software\Classes\Capstan.ssh" "CapstanOwner"
  ${If} $0 == "$INSTDIR\capstan-app.exe"
    DeleteRegKey HKCU "Software\Classes\Capstan.ssh"
  ${EndIf}
  Pop $0
!macroend
