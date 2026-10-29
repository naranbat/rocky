# Service

`rocky run` stays in the foreground until stopped. To keep it running in the background, start it at login and restart it if it crashes, let the system's service manager run it: systemd on Linux, launchd on macOS.

Install `rocky` first, and use its absolute path below, since service managers don't use your shell's `PATH`:

```sh
cargo install --path .     # installs ~/.cargo/bin/rocky
rocky validate /path/to/project.yaml
```

## Linux (systemd)

Create `~/.config/systemd/user/rocky.service`:

```ini
[Unit]
Description=rocky scheduler

[Service]
WorkingDirectory=/home/me/automation
ExecStart=/home/me/.cargo/bin/rocky run project.yaml --socket %t/rocky.sock
Environment=PATH=/home/me/.cargo/bin:/usr/local/bin:/usr/bin:/bin
Restart=on-failure
SuccessExitStatus=143

[Install]
WantedBy=default.target
```

- `WorkingDirectory` is where `.rocky/logs/` is written and where a relative project path is resolved.
- `PATH` is where the commands your tasks run (`curl`, `jq`, ...) are looked up.
- `%t` is `$XDG_RUNTIME_DIR`, e.g. `/run/user/1000`.
- `SuccessExitStatus=143`: `rocky` exits with 143 (128 + SIGTERM) when stopped, which systemd would otherwise mark as `failed`.

```sh
systemctl --user daemon-reload             # after creating or editing the unit
systemctl --user enable --now rocky        # start now and at every login
systemctl --user status rocky
journalctl --user -u rocky -f              # follow the log lines
systemctl --user restart rocky             # e.g. after changing a schedule
systemctl --user disable --now rocky       # stop, and don't start at login
rocky trigger $XDG_RUNTIME_DIR/rocky.sock report
```

A user service stops when you log out. To keep it running without a login session, and start it at boot:

```sh
loginctl enable-linger $USER
```

## macOS (launchd)

Create `~/Library/LaunchAgents/com.example.rocky.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.example.rocky</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/me/.cargo/bin/rocky</string>
    <string>run</string>
    <string>project.yaml</string>
    <string>--socket</string>
    <string>/tmp/rocky.sock</string>
  </array>
  <key>WorkingDirectory</key>
  <string>/Users/me/automation</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key>
    <string>/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>StandardOutPath</key>
  <string>/Users/me/automation/rocky.out.log</string>
  <key>StandardErrorPath</key>
  <string>/Users/me/automation/rocky.err.log</string>
</dict>
</plist>
```

- Paths must be absolute; launchd doesn't expand `~` or `$HOME`.
- `WorkingDirectory` and `PATH` work as on Linux.
- `KeepAlive` with `SuccessfulExit: false` restarts `rocky` only when it exits with an error.

```sh
plutil -lint ~/Library/LaunchAgents/com.example.rocky.plist                # check the file
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.example.rocky.plist   # start now and at every login
launchctl print gui/$(id -u)/com.example.rocky                              # status
tail -f /Users/me/automation/rocky.out.log                                  # follow the log lines
launchctl kickstart -k gui/$(id -u)/com.example.rocky                       # restart
launchctl bootout gui/$(id -u)/com.example.rocky                            # stop, and don't start at login
rocky trigger /tmp/rocky.sock report
```

After editing the plist, `bootout` and `bootstrap` it again.

## Notes

- **Stopping:** both service managers send SIGTERM, which stops `rocky` and the commands it runs.
- **Edits:** task and env file edits apply from the next run; schedule changes need a restart. See [Behaviour](run.md#behaviour). Run `rocky validate` before restarting: a project that doesn't load stops `run` at startup, and the service manager keeps retrying it.
- **Exit:** without `--socket`, `run` exits on its own once only finished `at` jobs are left. That is a successful exit, so neither example restarts it. With `--socket`, it runs until stopped.
- **Logs:** the same JSON lines go to the service manager's log (stdout) and to `.rocky/logs/YYYY-MM-DD.log` in `WorkingDirectory`. See [Output](run.md#output).
