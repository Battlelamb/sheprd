# Native file attachment cards

These macOS captures show the native chat renderer with a synthetic agent
reply containing standalone links to a video and a PDF. The filenames and
paths are sample data. No personal session or file contents are included.

- [Dark theme](dark.png)
- [Light theme](light.png)
- [Narrow window](narrow.png)

The opt-in `files` fixture uses the real transcript parser, card layout, and
native painter. Build with:

```sh
cargo build --locked -p sheprd-gpui --bin sheprd --features integration-test,mockup
```

Run the isolated fixture with `HERDR_TEST_RICH_CHAT=files`,
`HERDR_TEST_RICH_CHAT_CAPTURE=/absolute/output.png`, and `--sidebar-test`.
Set `HERDR_TEST_RICH_CHAT_THEME=light` for light mode, or
`HERDR_TEST_RICH_CHAT_SIZE=narrow` for the narrow window. Use private
`XDG_CONFIG_HOME` and `XDG_STATE_HOME` directories when capturing.

These screenshots verify the visible file cards and Open controls. They do
not exercise external media/PDF applications or attachment delivery to a live
agent. No fresh before image was captured. No VM or installed app was changed.
