## What and why

<!-- A short description of the change and the reason. Link the issue it closes, if any. -->

## Checks

- [ ] I ran: `npm run check`, `npm test`, `npm run build`, and in `src-tauri`: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
- [ ] README or the main repository's docs (`docs/guides/launcher.md`) updated if behaviour changed
- [ ] No game files, official-server captures, `Client.txt` excerpts, secrets, tokens or personal data in the change
- [ ] Nothing in the change reads or writes the Steam install of Path of Exile 2 or its Proton prefix, and nothing sends, alters or injects anything on official servers (the read-only rule)
- [ ] Passwords and tokens are not logged, not put in files or on command lines
