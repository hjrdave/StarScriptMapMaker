## StarScript Map Maker __VERSION__

__CHANGES__

### Downloads

| Platform | Package |
| --- | --- |
| Windows (x86_64) | `*-setup.exe` |
| macOS (Apple Silicon) | `*.dmg` |
| Linux (x86_64) | `*.deb`, `*.AppImage` |

SHA-256 checksums are in `SHA256SUMS.txt`. Check a download with
`sha256sum -c --ignore-missing SHA256SUMS.txt` (Linux),
`shasum -a 256 -c --ignore-missing SHA256SUMS.txt` (macOS) or `Get-FileHash <file>` (Windows).

### First launch: these builds are not signed

The packages are not code-signed or notarised, so your OS warns you the first time you run
them. Compare the checksums above if you want to be sure the file is the one this
repository's release workflow built.

**macOS (Gatekeeper)**

1. Open the DMG and drag StarScript Map Maker to Applications.
2. Open the app. macOS says it can't verify it; click **Done**.
3. Go to **System Settings → Privacy & Security**, scroll to the Security section and click
   **Open Anyway** next to StarScript Map Maker. Confirm with your password.

You only need to do this once.

**Windows (SmartScreen)**

1. Run the installer. SmartScreen shows "Windows protected your PC".
2. Click **More info**, then **Run anyway**.

**Linux**

No warning. For the AppImage, make it executable first: `chmod +x *.AppImage`.

### Updates

The app can check for new releases (opt-in). It reads `latest.json` from the latest release
and links to the download page; it never downloads or installs anything itself.
