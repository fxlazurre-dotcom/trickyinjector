# Tricky Injector

A minimal LoadLibrary injector for Minecraft Bedrock (Windows), written in Rust.
The UI runs on the system WebView2 through [wry](https://github.com/tauri-apps/wry)
— no bundled Chromium, the release binary is about 1 MB.

- Target process defaults to `Minecraft.Windows.exe` (editable in the UI)
- The DLL is picked through a native file dialog and validated as a 64-bit PE
  before it is ever used
- A manual **Inject** button plus an **Auto-inject** mode: the process is polled
  every 5 seconds and every new PID gets the DLL; already injected PIDs are
  skipped, so a game restart simply triggers a fresh injection
- Built for Bedrock **GDK** builds (1.21.120+): the game is a plain Win32
  process, no UWP/AppContainer tricks required
- The UI follows the *Dusk* theme of the [Hallmark](https://github.com/) design
  skill: dark slate, a smoked-glass panel with a lit top edge, one periwinkle
  action, apricot for waiting states

## Build

Requires the Rust `x86_64-pc-windows-msvc` toolchain from [rustup](https://rustup.rs).
The WebView2 Runtime ships with Windows 10/11.

```
cargo build --release
```

Binary: `target\release\tricky-injector.exe`

## How the injection works

1. `CreateToolhelp32Snapshot` finds the target PID by image name
2. `OpenProcess(PROCESS_CREATE_THREAD | PROCESS_VM_* | PROCESS_QUERY_INFORMATION)`
3. `VirtualAllocEx` + `WriteProcessMemory` write the DLL path into the target
   process (UTF-16, so non-ASCII paths are fine)
4. `CreateRemoteThread(LoadLibraryW)` loads the DLL inside the game
5. The thread exit code is the resulting HMODULE; `0` means LoadLibrary failed

## Project layout

| Path | What it is |
| --- | --- |
| `src/injector.rs` | WinAPI: process lookup, CreateRemoteThread injection, PE check |
| `src/file_dialog.rs` | `GetOpenFileNameW`-based DLL picker |
| `src/main.rs` | tao event loop, wry WebView, IPC, timers |
| `ui/` | The page inlined into the binary: `body.html`, `app.js`, `style.css`, `stamp.css`, `fonts.css` |
| `tokens.css`, `base.css` | Dusk theme tokens and base rules |
| `assets/fonts/` | Wix Madefor Display/Text, latin + cyrillic woff2 subsets |
| `preview.html` | Assembled single-file page used for lint runs (gitignored) |

## Notes

- The injector and the DLL must both be built for x64
- `OpenProcess ... error 5` — run the injector as administrator
- The DLL field rejects anything that is not an x64 PE right away
- `LoadLibraryW returned NULL` — the DLL or one of its dependencies (e.g. the
  MSVC runtime) failed to load inside the game
- Using modifications online may violate the game's rules — use at your own risk

## License

[MIT](LICENSE)
