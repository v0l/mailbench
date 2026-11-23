# Inline images

Paste an image (Ctrl+V or the platform paste key) or drop an image file on the composer and it goes into the message body at the cursor. A text paste is unchanged, and the image is only taken when the clipboard holds no text.

The body keeps a Markdown reference such as `![screenshot.png](cid:0123456789abcdef)`. The composer shows a thumbnail strip under the editor. Click a thumbnail, or delete the reference from the body, and the image is removed. The thumbnail is made once when the image is added.

Dropping other files adds normal attachments. Dropping on the mail view with no composer open does nothing.

## Sending

The HTML part carries `<img src="cid:...">` and the message is `multipart/alternative` with a plain text part that says `[image: name]`, and a `multipart/related` part holding the HTML and the images (Content-ID, Content-Disposition inline). Other attachments wrap everything in `multipart/mixed`.

With "send as plain text only" on, inline images are sent as normal attachments and you are told once.

## Rules

- Only PNG, JPEG, GIF and WebP are accepted, decided by the file bytes and not the name. SVG is refused because it can carry script.
- An image over 50 megapixels (width times height) is refused before it is decoded.
- An image over 5 MB is scaled down to at most 2000 pixels on the long edge and re-encoded: PNG stays PNG, photos become JPEG at quality 85. A note appears in the composer.
- A message whose encoded size is over 25 MB is refused before sending, with a message in the composer.
- Location data is always removed on send: EXIF and XMP are stripped from JPEG and WebP, text and EXIF chunks from PNG. Only the JPEG orientation tag is kept so the picture stays upright. There is no setting for this.

## Drafts, forward and reply

Inline images are stored with the local draft the same way attachments are, so autosave, reopening a draft and undo send keep them, and discarding a draft removes them. Forwarding keeps inline images inline. Replying quotes text only.

## Platforms

Desktop reads image data from the clipboard through the windowing layer (arboard). Android has no image paste yet: use the Attach key there, which adds a normal attachment. Drag and drop is desktop only.
