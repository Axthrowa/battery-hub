; Windows caches shell icons per executable path. An upgrade writes a new
; binary to the same path, so Explorer keeps drawing the previous icon on the
; shortcuts this installer has just created. Tell the shell its cache is stale.
!macro NSIS_HOOK_POSTINSTALL
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, i 0, i 0)'
!macroend
