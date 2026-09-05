# Remaining limits

- The v0.81.7 release already fails repository-wide code-size, test-size, panic, and swallowed-error ratchets. Pristine-versus-patched checks found no added failing paths or increased counts. The touched oversized production files were reduced, and the patch removed one silent error fallback. Do not claim all upstream CI gates pass.
- Existing TUI test compilation reports three warnings in unrelated UI/clipboard code. No warning was added in the changed production modules.
- Installing a new launcher does not replace already-running client processes. Preserve active sessions. The client-side scheduler fix takes effect when a new patched client is launched.
- A later upstream installation can replace the local build. Retain the commit, patch, and rollback receipt until the fix is integrated upstream.
