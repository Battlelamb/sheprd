# Native file attachment cards

These macOS captures show the native chat renderer with a synthetic agent
reply containing standalone links to a real synthetic video and PDF. Their
Quick Look thumbnails show a video frame and the first PDF page. No personal
session or file contents are included.

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

These screenshots verify the visible file cards, native thumbnails, and Open
controls. They do not exercise external media/PDF applications or attachment
delivery to a live agent. The same sample conversation before thumbnails is
available in the [previous native capture](https://github.com/sheprd-labs/sheprd/blob/2c2101953199ee796a8c1170bf8a19cb4d639c99/docs/screenshots/chat-file-attachments/dark.png).
No VM or installed app was changed.

Thumbnails are generated only for local PDF/video attachments on macOS.
Missing/unsupported files and remote attachments retain a generic file icon.
The native test uses the actual Quick Look service:

```sh
cargo test --locked -p sheprd-gpui --lib native_pdf_and_video_thumbnails_are_real_bounded_images -- --ignored
```
