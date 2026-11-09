# Lampfield

A small desktop app for Windows and macOS that shows every Claude Code session from the Claude desktop app as a plain tile, grouped the same way as your Claude sidebar.

The name comes from the busy lamp field on an office phone: a row of lights, one per colleague, that shows who's busy at a glance without anyone moving.

In each group, the session you last messaged comes first. A tile only moves when you start a session or reply to one, never because it finished or asked you something. Otherwise each tile stays in its spot and changes how it looks:

| Tile | Means |
|---|---|
| Amber ring, **?** | Needs you: a question, a plan to approve, or a reply it's waiting for |
| Dashed amber, **!** | May need you: a tool has been pending a while, which is usually an approval prompt |
| Green, **✓** | Finished since you last looked |
| Red, **×** | Stopped on an error |
| Moving line along the bottom | Working |
| Faded | Idle |

Click a tile to open that session in Claude. Right-click it for its panel:

- **Now**: what it's doing or what it needs, including a question's answer options
- **Changes**: the files it changed, with lines added and removed
- **Steps**: its last few actions

The panel also has:

- **Open in Claude**
- **Dismiss**, which quietens an alert until something new happens in that session, or clears a green tile
- **Hide**, which takes the tile off the board

Hidden sessions sit under **Hidden** at the top, which turns amber if one of them needs you. From there you can **Show** a session again, or click **Archive in Claude**. That opens a new Claude session with a prompt asking it to archive them all, because only Claude can archive its own sessions. The count buttons at the top dim the other tiles rather than reordering them. The tray icon shows when something needs you. On Windows, left-click it to open the board and right-click it for the menu.

## Install on Windows

1. On GitHub, open the repo's **Actions** tab, then the latest **Windows build** run, and download **lampfield-windows** from Artifacts. If you pushed a tag like `v0.1.0`, the installer is also on the Releases page.
2. Unzip it and run `Lampfield_0.1.0_x64-setup.exe`. It installs for your user only, so it doesn't need admin rights.
3. The installer isn't signed, so Windows SmartScreen may say it "protected your PC". Click **More info**, then **Run anyway**. If your IT policy blocks unsigned apps completely, that step won't work and the app would need signing.
4. Pin the tray icon so it doesn't hide in the overflow: drag it from the **^** menu onto the taskbar, or turn it on in Settings, Personalisation, Taskbar, Other system tray icons.

If the board says it found no sessions, it shows the folder it looked in. It expects Claude's data in `%APPDATA%\Claude`. The Microsoft Store version of Claude keeps it under `%LOCALAPPDATA%\Packages\...\LocalCache\Roaming\Claude`, which is also checked. If yours is somewhere else, set the `LAMPFIELD_CLAUDE_DIR` environment variable to the folder that holds `claude-code-sessions`.

## How it works

The Claude desktop app keeps a small JSON record per Claude Code session in `claude-code-sessions`, and mirrors your sidebar groups into `claude_desktop_config.json`. Claude Code writes a transcript of each session under `~/.claude/projects`. Lampfield watches those files and works out each session's state from them. It only ever reads them. It has no network access and sends nothing anywhere. Opening a session uses Claude's own `claude://` links.

These are Claude's internal formats, not a public API, so a Claude update can break the app. The reading code is in `crates/lampfield-core`.

Limits worth knowing:

- It can't answer a session for you. Claude doesn't let other apps do that, so the panel shows the question and you answer in Claude.
- Approval prompts aren't written to the transcripts. "May need you" is a guess, based on a tool being pending for more than 45 seconds.
- Your seen, dismissed and hidden choices are stored in this app's own data folder, never in Claude's.

## Develop

Needs Node 20+ and Rust stable. On Windows you also need the Visual Studio C++ build tools. WebView2 comes with Windows 11.

```bash
npm install
npm test                                   # page logic and Rust tests
npm run dev                                # the page in a browser, with made-up sessions
npm start                                  # the app, on your real sessions
cargo run -p lampfield-core --example check    # where it looks and what it finds, as counts only
npm run bundle                             # macOS .app
npx tauri build --bundles nsis             # Windows installer, run on Windows
```

`tools/make-assets.py` draws the icons and the chime. GitHub Actions (`.github/workflows/windows.yml`) builds the Windows installer on every push to `main`.

