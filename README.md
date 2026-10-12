# NextCue

**Your desktop companion for conversations, meeting notes, interview practice, and learning from videos.**

NextCue transcribes audio you choose locally and helps turn conversations into useful answers and notes. Use local AI through Ollama, or connect a supported ChatGPT or Claude account. A floating assistant keeps your current session within reach while you work.

Created by **Fidele Kirezi Cyisa**. NextCue is an early desktop product under active development, built with Tauri, Rust, React, and TypeScript.

![NextCue Home](docs/screenshots/home.png)

## What it does

| Workflow | Features |
| --- | --- |
| Interviews and practice | Add your role, background, and job description; ask questions or enable automatic answer suggestions. |
| Meetings | Capture transcripts and generate decisions, action items, and open questions. |
| Browser videos | Capture playing audio, create study notes, and export timed SRT/WebVTT subtitles. |
| Floating assistant | Access answers and captions, pause listening, and use configurable shortcuts. |
| Session library | Review saved transcripts and answers, create recaps, and export Markdown. |
| Background work | Keep sessions running through the tray; optionally enable meeting hints or pair the browser companion. |

### Create a session

Choose Interview, Practice, Meeting, or Video and supply context to guide the assistant.

![Session creation with sample title and role](docs/screenshots/new-session.png)

### Choose your AI and audio

Ollama is the default answer provider. Whisper transcribes speech locally. Settings also supports optional account providers and Jev context help.

![NextCue AI and audio settings](docs/screenshots/settings.png)

These screenshots show the actual web preview with sample session fields. Native audio, AI connections, and local file storage require the desktop app.

## Run locally

Install Node.js/npm, Rust, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). For local answers, install [Ollama](https://ollama.com/) and download a model:

```sh
ollama pull qwen2.5:7b-instruct
npm install
npm run dev
```

In Settings, check your AI connection and choose **Set up transcription** to download Whisper Base once (about 148 MB). Create a session, select your audio input, and start listening. Manual questions do not require transcription setup.

macOS system audio requires macOS 13 or newer and Screen & System Audio Recording permission. Microphone input requires microphone permission. Windows/Linux call or browser audio requires a selected loopback or virtual audio input.

For interface development only, run `npm run dev:web`.

## Privacy and current limits

- With Ollama selected and Jev disabled, transcription and AI inference stay local. Initial model downloads require internet access.
- Connected ChatGPT/Claude providers receive questions and relevant session context. Jev sends context to TypeSafe. Provider terms and usage limits apply.
- Sessions are saved locally as plain JSON. Audio is processed in memory rather than saved as recordings. NextCue credentials use the OS credential store; Claude Code manages its own login.
- Listening starts from an explicit action or a meeting action you enabled. Meeting hints default to off and indicate app/page presence, not a verified active call.
- Answers and transcripts can contain errors. Speaker diarization, PDF/DOCX resume parsing, and screen-image analysis are outside this version.
- Subtitle timing assumes continuous playback at normal speed. Captions appear in NextCue and export as files; they are not embedded in the source video.
- Visible windows may appear in screen sharing. Use the background/hide workflow and check your sharing preview.

macOS is the current development platform. Windows/Linux native behavior and live paid-account inference still need verification. Commercial distribution with ChatGPT subscription sign-in requires separate provider approval; this project's license does not grant it.

See the [full setup and usage guide](docs/USER_GUIDE.md) for account setup, shortcuts, permissions, exports, and platform limits, and the [browser companion guide](browser-extension/README.md) for optional pairing.

## Build and verify

```sh
npm run build
npm test
```

With the web preview running in another terminal and Chrome installed, run `npm run test:ui`.

Build on the target operating system:

```sh
npm run package:mac
# Windows: npm run package:windows
# Linux: npm run package:linux
```

Distribution requires appropriate platform signing/notarization.

## Ownership and license

Copyright © 2026 **Fidele Kirezi Cyisa**. All rights reserved.

NextCue's original code, documentation, and assets are proprietary under the [LICENSE](LICENSE). Use, modification, redistribution, and commercial reuse require the author's written permission, except where applicable law permits otherwise. This is not an open-source license. Third-party components retain their own licenses and terms.

This notice reserves rights; it does not transfer third-party ownership or replace contributor agreements. Open-source licenses also preserve copyright ownership, but grant reuse rights. See [reserved rights explained](https://choosealicense.com/no-permission/) and the [U.S. Copyright Office ownership overview](https://www.copyright.gov/what-is-copyright/).
