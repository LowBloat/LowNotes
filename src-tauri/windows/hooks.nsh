; Removes the "Open with LowNotes" entries that the app writes when its
; preference is on (src-tauri/src/context_menu.rs). Updates keep them.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegKey HKCU "Software\Classes\Directory\shell\LowNotes"
    DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\LowNotes"
    DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.md\shell\LowNotes"
    DeleteRegKey HKCU "Software\Classes\SystemFileAssociations\.markdown\shell\LowNotes"
  ${EndIf}
!macroend
