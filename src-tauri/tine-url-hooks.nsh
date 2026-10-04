; GH #181: only Tine's own open-only scheme; no Logseq protocol takeover.
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\tine" "" "URL:Tine navigation"
  WriteRegStr SHCTX "Software\Classes\tine" "URL Protocol" ""
  WriteRegStr SHCTX "Software\Classes\tine\shell\open\command" "" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"'
!macroend
!macro NSIS_HOOK_PREUNINSTALL
  ReadRegStr $0 SHCTX "Software\Classes\tine\shell\open\command" ""
  StrCmp $0 '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\"' 0 +2
  DeleteRegKey SHCTX "Software\Classes\tine"
!macroend
