; Shell may retain the opaque icon from an older EdgeMouse executable after
; in-app upgrades. Notify it after replacement, without restarting Explorer
; or deleting any system/user icon-cache files.
!macro NSIS_HOOK_POSTINSTALL
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
