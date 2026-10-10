# NextCue

A desktop assistant for interviews, practice, and meetings, with a Rust/Tauri core and native Whisper speech recognition. Choose local Ollama or an existing ChatGPT/Claude account in Settings.

## What works

- Create interview, practice, or meeting sessions with your role, resume text, job description, and answer instructions.
- Stream answer suggestions from your installed Ollama models. No API key or AI subscription is required.
- Optionally connect an eligible ChatGPT account through OpenAI's official sign-in flow, or use your own installed Claude Code subscription login. Answers stream inside NextCue and the floating pin.
- Use session mode, role, company, background, and recent transcript as context. Meeting notes distinguish decisions, action items, and open questions; interview prompts require supported personal facts.
- Optionally enable Jev context hints with your TypeSafe API key. Jev makes structured context decisions; your selected answer provider writes the response.
- Transcribe microphone input with local Whisper Base. On macOS, capture call/system audio through ScreenCaptureKit, including calls heard through headphones.
- Detect questions and suggest answers automatically, or submit questions manually with Cmd/Ctrl+Enter.
- Start from a focused Home screen, browse your session library, and set up AI/audio in two steps.
- Pause listening, cancel generation, switch sessions, open a floating assistant pin, and end sessions.
- Keep local session history, review answers, generate recaps, import UTF-8 text/Markdown/JSON context, and export a session to Markdown.

## Run

Requirements: Node.js, Rust, and the platform build tools listed in [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). Local answers require an installed Ollama model. macOS system audio requires macOS 13 or newer.

```sh
npm install
npm run dev
```

Start Ollama in Settings, select a locally installed model, and create a session. The default is `qwen2.5:7b-instruct`; your other installed models are available in the selector. If needed:

```sh
ollama pull qwen2.5:7b-instruct
```

Manual questions work as soon as Ollama is connected. For voice, choose **Set up transcription** in Settings. This downloads the 148 MB multilingual Whisper Base model once, verifies its SHA-256 digest, and stores it locally. Later speech recognition uses those local weights without an internet connection.

## Existing ChatGPT and Claude accounts

Choose **Settings → Your AI → Answer provider**. Local AI remains the default. Changing provider does not move existing sessions or import provider chat history.

**ChatGPT:** choose **Continue with ChatGPT**, finish OpenAI's sign-in and consent in your browser, then return to NextCue. Approve ChatGPT plan usage to receive answers inside the app; identity sign-in alone does not grant inference access. Select a model from the account's live catalog using **Check connection**. **Manage usage** opens ChatGPT's usage controls. Saved accounts can be selected, reconnected, or disconnected. The integration uses dynamic client registration, loopback callback, PKCE, verified ID tokens, and native OS credential storage. No OpenAI API key is needed. Subscription availability and account usage limits apply. OpenAI currently allows this flow for local personal projects and open-source tools; a paid or remotely hosted offering needs [OpenAI approval](https://developers.openai.com/cookbook/articles/sign-in-with-chatgpt). This local build does not imply approval to distribute a commercial subscription integration.

**Claude:** install the published [Claude Code](https://code.claude.com/docs/en/setup) CLI and sign in through Anthropic's own flow in a terminal:

```sh
claude auth login
```

Choose Claude in NextCue and **Check connection**. NextCue invokes that unmodified installation for answers, with tools and project customizations disabled and session persistence off. It requires Claude account authentication and does not silently fall back to API/cloud billing. NextCue never reads or stores Claude credentials. Claude plan limits apply. This is the installed Claude Code route allowed in [Anthropic's terms](https://code.claude.com/docs/en/legal-and-compliance), rather than a custom Claude OAuth login.

**Jev:** under **Optional context help with Jev**, save a key from the [TypeSafe console](https://console.typesafe.ai/) and enable context hints. The key is stored in the operating system credential store, never in session settings. Jev checks a question's context through the documented TypeSafe API and can suggest a clarification or meeting notes. It does not guarantee correctness or infer a meeting from another app. Low-confidence results are advisory; a failed check still lets the chosen answer provider continue. TypeSafe has its own usage allowance, separate from ChatGPT/Claude subscriptions. Removing the key disables the helper.

In the session input selector, choose a microphone or **Mac system audio**. Grant Microphone or Screen & System Audio Recording permission in macOS System Settings when prompted. Restart the app if macOS asks. System capture includes computer audio; avoid unrelated audio playing during a call. Windows/Linux can route call output to a virtual/loopback input device and select that input.

## Floating assistant

Open a session and choose **Pin mode** to shrink NextCue to a movable assistant button. Drag its small grip to place it beside your call. Click the button to reveal the latest answer, or generate an answer to the latest unanswered question. **Get answer** can generate another suggestion; the microphone button starts or pauses listening. **Open full app** restores your workspace.

The pin and full workspace share the same live session: switching modes preserves listening, streamed answers, and saved history. Transcript and background are available as optional side panels in the full workspace. Add personal context during session creation under **Personalize answers**, or edit it later.

Use **Cmd+Shift+Space** on macOS or **Ctrl+Shift+Space** on Windows/Linux to show or collapse the answer card when the operating system allows shortcut registration. **Cmd/Ctrl+Shift+H** hides or shows the assistant without stopping listening or answer generation. Edit these bindings in **Settings → Assistant & screen sharing**, then choose **Save shortcuts**. Shortcut text uses combinations such as `CommandOrControl+Alt+A`; invalid or conflicting registrations do not replace your saved bindings. The default hide/show binding remains available as a recovery shortcut when registration succeeds. The pin position is remembered locally and expanded answers are kept within the display's work area.

Choose **Exclude assistant from screen capture when supported** in Settings to request Windows capture exclusion, reported only as **best effort**. Turn it off to include the visible assistant in a recording. macOS/Linux report **May appear in screen sharing** while exclusion is enabled because they cannot reliably exclude the window. **Hide assistant** hides the native window and removes its interface content from the webview while session work continues. The hide/show shortcut brings it back. Hiding removes the window from the visible desktop; it cannot erase earlier footage, control a recorder's cached frames, or guarantee exclusion from every recording method. These controls do not stop a recording or a screen share, and do not hide the system pointer. Verify the actual recording/share preview. Sharing a specific application window or using an unshared monitor can keep the assistant separate when those options are available. Pin/focus/shortcut behavior needs native verification on Windows/Linux; Linux window-manager restrictions can limit placement.

In the focused full app, **Tab / Shift+Tab** moves between controls and **Enter / Space** activates buttons. Focus the answer region to scroll with arrow keys or **Page Up / Page Down**. **Cmd/Ctrl+Enter** answers the current question only on the live Answer tab, with no dialog open. There is no concealed mouse or recorder-wide cursor control; keyboard navigation does not change the recorder's capture settings.

## Build a desktop application

```sh
npm run package
```

The default packaging command uses the current operating system's bundle targets. Explicit commands are `npm run package:mac` (app), `npm run package:windows` (NSIS), and `npm run package:linux` (AppImage and deb). Build each on its target operating system with Tauri's prerequisites. The macOS app is built at `src-tauri/target/release/bundle/macos/NextCue.app`. Windows/Linux native builds have not been verified in this macOS workspace. For a faster development bundle:

```sh
npx tauri build --debug --bundles app
```

This is a local development app. Distribution to other computers requires normal platform signing/notarization. Rust implements the application core; `whisper-rs` binds to the native Whisper engine, and the UI uses React/TypeScript in the operating system's webview.

## Privacy and storage

With local AI and Jev disabled, prompts and inference remain on this computer. The app accepts only loopback Ollama addresses and rejects cloud model tags and remote-backed aliases. Choosing ChatGPT or Claude sends the question, supplied background, session metadata, and relevant transcript to that provider. Enabling Jev sends the same context to TypeSafe for a structured check. Speech transcription stays local for every provider. Listening starts only on user action. The initial Whisper download requires internet access. Audio segments are held in bounded memory queues and are not saved as recordings.

Settings and sessions are written atomically to the app data directory, normally `~/Library/Application Support/com.nextcue.desktop` on macOS. Whisper weights are in its `models` subdirectory. Transcript timestamps use the actual audio/question time. Local records are plain JSON; exports are plain Markdown. ChatGPT tokens and the optional TypeSafe key use the OS credential store. Claude Code owns its own authentication. Provider account access must be completed by the user; development verification does not log into accounts or consume subscription allowance.

For isolated QA, set `NEXTCUE_DATA_DIR` to an absolute directory before launching. This redirects settings and model storage. It does not change the user's Ollama model library.

## Verification

```sh
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

The UI tests exercise persistence, streaming cancellation across sessions, preserving the final audio segment after stopping, truthful browser-preview behavior, and companion mode continuity for listening, answers, shortcuts, and recaps. They inject a test-only desktop bridge. Rust tests exercise the actual service parsing, local-only restrictions, storage, native audio processing, and companion placement. Start a separate UI server and use installed Chrome:

```sh
npm run dev:web
npm run test:ui
```

Ignored integration tests can run against real local Ollama and a local Whisper model; see the tests in `src-tauri/src/ollama.rs` and `src-tauri/src/voice.rs` for their environment variables. The browser preview is for interface development; Ollama, native audio, file dialogs, and local app storage run in the desktop application.

## Current limits

Whisper processes short audio segments, so transcript/answer latency depends on the model and hardware. There is no speaker diarization or automatic Zoom/browser meeting detection yet; choose Meeting mode to get the note-taking suggestion. PDF/DOCX resume parsing, screen/coding-image analysis, billing, and app distribution are outside this first version. AI suggestions can be incorrect; review them and use your own experience. This app does not claim to be hidden from screen sharing. Live ChatGPT/Claude/Jev inference requires the user's connected account/key and has not been verified with a paid account in this workspace.

Primary references: [Ollama chat API](https://docs.ollama.com/api/chat), [Tauri](https://v2.tauri.app/), [whisper-rs](https://docs.rs/whisper-rs/0.15.1/whisper_rs/), [ScreenCaptureKit Rust bindings](https://docs.rs/screencapturekit/11.0.0/screencapturekit/).
