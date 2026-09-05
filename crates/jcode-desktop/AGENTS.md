# Jcode Desktop Agent Context

- This is the Jcode desktop application crate. Desktop-launched agents opening here assume desktop self-development unless the user says otherwise.
- Prefer targeted desktop checks while iterating: `cargo check -p jcode-desktop` and relevant `jcode-desktop` tests.
- Scope changes to desktop UI/session-launch code when possible. Update shared crates when the desktop implementation requires it.
- App-launched desktop sessions default here so local `AGENTS.md` primes agents for desktop self-dev.
