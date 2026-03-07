!macro NSIS_HOOK_POSTINSTALL
  ; Register extension in Google Chrome
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "path" "$INSTDIR\resources\extension"
  WriteRegStr HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "version" "1.0"

  ; Register extension in Microsoft Edge
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "path" "$INSTDIR\resources\extension"
  WriteRegStr HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk" "version" "1.0"
!macroend

!macro NSIS_HOOK_UNINSTALL
  ; Clean up registry on uninstall
  DeleteRegKey HKCU "Software\Google\Chrome\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk"
  DeleteRegKey HKCU "Software\Microsoft\Edge\Extensions\npnahejfadhjkhgnhciecngenjmkcbkk"
!macroend
