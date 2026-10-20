# Signing and verifying ORAG releases

ORAG's release workflow (`.github/workflows/release.yml`) builds every file on
GitHub's runners from a pushed tag. This page explains how a download can be
checked today, and what turns on code signing for the desktop app.

## What every release has now

- **`SHA256SUMS`**: one SHA-256 line per release file. After downloading, check
  a file against it:

  ```sh
  # macOS
  shasum -a 256 -c SHA256SUMS --ignore-missing
  # Linux
  sha256sum -c SHA256SUMS --ignore-missing
  ```

  ```powershell
  # Windows (PowerShell): compare with the line for the file in SHA256SUMS
  Get-FileHash .\ORAG_0.4.1_x64-setup.exe -Algorithm SHA256
  ```

- **Build provenance**: GitHub signs a statement that each file was built by
  this repository's release workflow from the tag. Verify it with the GitHub CLI:

  ```sh
  gh attestation verify ORAG_0.4.1_x64-setup.exe --repo orkono/orag
  ```

  It fails for a file that was changed after the build or built anywhere else.

Neither of these removes the operating system's warning for an unsigned app;
they prove the file is the one this repository built.

## macOS: Developer ID signing and notarization

Signing removes "the developer cannot be verified". It needs a membership in
the Apple Developer Program (paid yearly, individual or organization; an
organization needs a D-U-N-S number).

Once the membership is active:

1. In Xcode or on developer.apple.com, create a **Developer ID Application**
   certificate and export it with its private key as a `.p12` file with a
   password.
2. Create an **app-specific password** for the Apple ID at appleid.apple.com.
3. Add these repository secrets (GitHub → Settings → Secrets and variables →
   Actions):

   | Secret | Value |
   |---|---|
   | `APPLE_CERTIFICATE` | the `.p12` file, base64 encoded (`base64 -i cert.p12`) |
   | `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
   | `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
   | `APPLE_ID` | the Apple ID e-mail |
   | `APPLE_PASSWORD` | the app-specific password |
   | `APPLE_TEAM_ID` | the 10-character team ID |

When `APPLE_CERTIFICATE` is set, the macOS desktop build signs and notarizes the
`.dmg` (Tauri reads these variables). If it is set but another secret is
missing, the build stops with the missing name. Without the secrets the app is
built unsigned, as before. The first signed release should be checked with
`spctl -a -vv /Applications/ORAG.app` and `codesign -dv --verbose=4` on the
bundled `orag` service binary.

The CLI archives (`orag-<version>-*.tar.gz`) are not signed by this step.

## Windows: Authenticode

Signing removes the "unknown publisher" text. SmartScreen can still warn for a
new certificate until its downloads build a reputation. Options, to be chosen
by the owner (check each provider's current terms and eligibility first):

- **SignPath Foundation**: free code signing for open-source projects, after an
  application.
- **Azure Trusted Signing**: a monthly subscription; keys stay in Azure and the
  workflow signs through it.
- **An OV certificate from a certificate authority**: the key must live in a
  hardware token or a cloud HSM.

The workflow has no Windows signing step yet; it is added once a provider is
chosen, for both `ORAG_<version>_x64-setup.exe` and `ORAG-Setup-<version>.exe`.

## Linux

AppImage and `.deb` files have no operating-system signature check. Use
`SHA256SUMS` and `gh attestation verify` above.
