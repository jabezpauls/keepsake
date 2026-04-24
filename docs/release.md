# Cutting a Keepsake release

The release pipeline lives in `.github/workflows/release.yml`. It builds
installers for Linux x86_64, macOS Intel, macOS Apple Silicon, and
Windows x86_64 in parallel, then attaches them to a draft GitHub Release
named after the pushed tag.

## Quickstart — unsigned release

```sh
# 1. Bump version in app/src-tauri/tauri.conf.json + Cargo.toml workspace.
# 2. Commit + tag.
git tag v0.1.0
git push origin v0.1.0
```

That's it. The workflow:

1. Spins up the matrix (Linux + 2× macOS + Windows).
2. Downloads the matching ONNX Runtime binary
   (`onnxruntime-{linux-x64,osx-x86_64,osx-arm64,win-x64}-1.22.0`) and
   stages `libonnxruntime` into `app/src-tauri/resources/`.
3. Runs `cargo tauri build --features ml-models --target <triple>`,
   producing a `.dmg` / `.deb` / `.AppImage` / `.rpm` / `.msi` / `.exe`
   per OS.
4. Creates a **draft** GitHub Release with every artifact attached.
5. Maintainer reviews the draft + clicks "Publish".

Manual triggers via `workflow_dispatch` skip the release step and upload
artifacts to the workflow run instead — useful for validating a branch
before tagging.

## Release artifact map

| OS | Tauri targets | Recommended download |
|---|---|---|
| Linux x86_64 | `.AppImage`, `.deb`, `.rpm` | `.AppImage` (no install needed) |
| macOS Intel | `.app.tar.gz`, `.dmg` | `.dmg` |
| macOS Apple Silicon | `.app.tar.gz`, `.dmg` | `.dmg` |
| Windows x86_64 | `.msi` (WiX), `.exe` (NSIS) | `.msi` |

Each binary bundles a CPU-only `libonnxruntime` (~15 MB compressed). On
first launch the app's wizard offers to download the on-device AI model
weights — Lite (~790 MB) for CPU hosts, Full (~1.5 GB) for GPU hosts.

## Going public — code-signing setup

Without signing:

- macOS users see "Keepsake.app is from an unidentified developer" and
  must right-click → Open to bypass Gatekeeper.
- Windows users see "Microsoft Defender SmartScreen prevented an
  unrecognized app from starting" and must click "More info" → "Run
  anyway".
- Linux is unaffected (no platform signing infrastructure).

Once you're ready to ship signed builds, add these GitHub Actions
secrets to the repository (`Settings → Secrets and variables →
Actions`):

### macOS (Apple Developer Program — $99/year)

| Secret | Description |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` "Developer ID Application" cert exported from Keychain Access. `base64 -i cert.p12 \| pbcopy` on macOS. |
| `APPLE_CERTIFICATE_PASSWORD` | The password you set when exporting the `.p12`. |
| `APPLE_SIGNING_IDENTITY` | Your "Developer ID Application: Your Name (TEAMID)" string (find via `security find-identity -v -p codesigning`). |
| `APPLE_ID` | Your Apple ID email. |
| `APPLE_PASSWORD` | An app-specific password generated at appleid.apple.com (NOT your Apple ID password). |
| `APPLE_TEAM_ID` | The 10-character Team ID from your Apple Developer account. |

`tauri-action` reads all six and runs `codesign` + `notarytool` for you.
First notarization run takes ~10 minutes per artifact while Apple
verifies; subsequent runs are faster.

### Windows (Authenticode — ~$200/year)

The cheapest reputable Authenticode certs come from SSL.com or
Certum. Once you have one:

| Secret | Description |
|---|---|
| `WINDOWS_CERTIFICATE` | Base64-encoded `.pfx` cert. |
| `WINDOWS_CERTIFICATE_PASSWORD` | The cert export password. |

Then add a step in `.github/workflows/release.yml` after `tauri-action`:

```yaml
- name: Sign Windows MSI
  if: runner.os == 'Windows'
  shell: pwsh
  env:
    CERT: ${{ secrets.WINDOWS_CERTIFICATE }}
    CERT_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}
  run: |
    [IO.File]::WriteAllBytes("cert.pfx", [Convert]::FromBase64String($env:CERT))
    Get-ChildItem -Path 'target/${{ matrix.target }}/release/bundle/msi/*.msi' |
      ForEach-Object {
        & "$env:WindowsSdkDir\bin\10.0.22621.0\x64\signtool.exe" sign `
          /f cert.pfx /p $env:CERT_PASSWORD `
          /fd SHA256 /tr http://timestamp.sectigo.com /td SHA256 `
          $_.FullName
      }
```

(Tauri 2's bundler can also sign in-flight via `bundle.windows.signCommand`
in `tauri.conf.json` — pick whichever fits your secret rotation.)

## Versioning

Keepsake follows [SemVer](https://semver.org/) starting at v0.x. Until
v1.0 is cut:

- **Patch** (v0.1.1): bug fixes, no new features, no schema bumps.
- **Minor** (v0.2.0): new features, additive schema migrations, no
  breaking IPC changes.
- **Major-pre-1.0** (v0.x.0): can include breaking changes — call them
  out in the release notes.

Once v1.0 ships, breaking changes require a major bump.

## Verifying a draft release

Before publishing the draft, smoke-test at least one artifact per OS:

1. Download the `.AppImage` / `.dmg` / `.msi`.
2. Install / launch.
3. Create a fresh user.
4. Add a small source directory.
5. Open the model wizard (badge → "ML — download models"), pick the
   bundle that matches your hardware, complete the download.
6. Confirm the badge flips to "ML Cpu · idle" (or `Cuda` / `CoreMl`).
7. Run a search; confirm semantic results come back.

If anything fails, delete the draft and iterate — a draft release
doesn't notify watchers or trigger update channels.

## Auto-updater (future)

Tauri has a built-in updater that consumes a signed `latest.json`
manifest. Wiring it up requires:

1. A signing keypair (`tauri signer generate`).
2. Adding `tauri-plugin-updater` to `app/src-tauri/Cargo.toml`.
3. A new workflow step that publishes `latest.json` alongside the
   artifacts.
4. UI in the shell to surface "update available" notifications.

Defer until after v0.1.0 ships and download patterns stabilise.
