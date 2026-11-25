# Controlled startup failure (TX-02)

Only `--tx00-fail-window-create` enables the fixture; ordinary launches do not
register a rejecting class. This flag is for isolated verification, not a
product feature. No system settings, drivers, existing windows or user files
are modified by the fixture.

Before the unchanged NSH run_app call, the fixture registers process-local
NshWindow under the executable's module, with a callback rejecting WM_NCCREATE
and setting ERROR_ACCESS_DENIED. The pinned backend attempts to register that
same class, then its real CreateWindowExW calls this callback and fails.
AppView::make's actual error propagates through the backend's existing
`Cannot open a window` -> fatal_error -> ExitCode::FAILURE path. Registration
failure while arming is a setup error, not accepted as NSH failure evidence.

NSH shows a modal MessageBox. It must be acknowledged before exit. The test
script matches the exact test PID, class #32770 and title nsh, records the
dialog, then clicks its sole enabled visible Button with BM_CLICK, records
the actual control ID, and observes that the
dialog closed. It requires nonzero process exit within
15 seconds after acknowledgement, no residual PID, and both the callback
marker and NSH diagnostic in stderr. It does not claim automatic unassisted
exit or treat time waiting at the dialog as a hang.

```powershell
./scripts/test-startup-failure.ps1 -Exe ./dist/package/texzi-app.exe -EvidenceDir ./startup-failure-new
```

Use a new evidence directory. Four generated fixtures become the child working
directory; compare their relative paths, byte lengths, SHA256 and UTC mtime
before/after. stderr, stdout, executable hash, dialog/exit timing and snapshots
are retained outside the fixture directory. Forced cleanup of a remaining test
PID is recorded and the test fails. Never point this at user documents.

Coverage: the pinned NSH Win32 window-creation error and its real modal
diagnostic/exit path. Not covered: UI-thread spawn failure, COM failure, GPU or
DirectWrite failure after successful window creation, framework engine panic,
all OS loader failures, global filesystem/registry non-writing, or transient
write-and-restore behavior. Normal NSH identity/cache effects remain possible.
The callback relies on the pinned backend's class name and implementation;
revalidate it when changing NSH. No upstream source patch is introduced.
