!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\uninstall.exe" 0 done
    MessageBox MB_YESNO|MB_ICONQUESTION "Velocity Download Manager is already installed.$\r$\n$\r$\nChoose Yes to repair / reinstall it.$\r$\nChoose No for uninstall or cancel options." IDYES reinstall
    MessageBox MB_YESNO|MB_ICONQUESTION "Do you want to uninstall Velocity Download Manager instead?$\r$\n$\r$\nChoose Yes to uninstall only.$\r$\nChoose No to cancel setup." IDYES uninstall IDNO cancel

  uninstall:
    IfFileExists "$INSTDIR\uninstall.exe" 0 cancel
    ExecWait '"$INSTDIR\uninstall.exe"'
    Abort

  cancel:
    Abort

  reinstall:
  done:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Register extension in Google Chrome
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "path" "$INSTDIR\resources\extension"
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "version" "2.0.0"

  ; Register extension in Microsoft Edge
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "path" "$INSTDIR\resources\extension"
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "version" "2.0.0"
!macroend

!macro NSIS_HOOK_UNINSTALL
  ; Clean up registry on uninstall
  DeleteRegKey HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk"
  DeleteRegKey HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk"
!macroend
