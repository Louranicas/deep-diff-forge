# Review cockpit collaboration records (2026-06-23)

These two files arrived from GitLab `main`, where three commits (`82adeba`, `30937ba`, `e689f9c`)
were made on 2026-06-23 and never reached GitHub. Their code had already landed on GitHub `main`
by another path: after the reconciliation merge of 2026-10-27, `crates/` is byte-identical to
GitHub `main` before it. The merge joins the two histories so GitLab `main` can fast-forward.

The records are kept unedited, as dated history. Paths in them are from the machine they were
written on and are not instructions. Current behaviour is documented in the README and `docs/`.

- [COLLAB_LOG.md](COLLAB_LOG.md): the multi-agent integration log for the cockpit's
  performance, mouse, menu and CLI lanes.
- [TUI_PROBE_HANDSHAKE.md](TUI_PROBE_HANDSHAKE.md): the headless `review --probe` handshake.
