# NextCue

A desktop assistant for interviews, practice, meetings, and video notes or subtitles, with a Rust/Tauri core and native Whisper speech recognition. Choose local Ollama or an existing ChatGPT/Claude account in Settings.

## What works

- Create interview, practice, or meeting sessions with your role, resume text, job description, and answer instructions.
- Stream answer suggestions from your installed Ollama models. No API key or AI subscription is required.
- Optionally connect an eligible ChatGPT account through OpenAI's official sign-in flow, or use your own installed Claude Code subscription login. Answers stream inside NextCue and the floating pin.
- Use session mode, role, company, background, and recent transcript as context. Meeting notes distinguish decisions, action items, and open questions; interview prompts require supported personal facts.
- Optionally enable Jev context hints with your TypeSafe API key. Jev makes structured context decisions; your selected answer provider writes the response.
- Transcribe microphone input with local Whisper Base. On macOS, capture call/system audio through ScreenCaptureKit, including calls heard through headphones.
- Capture a playing YouTube or other browser video in a Video session, show live speech captions, generate study notes, and export timed SRT/WebVTT subtitles.
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

## Notes and subtitles from browser videos

Choose **Video** from Home or the new-session dialog. Give the session a title and optionally add the video topic or what you want to learn. If you are starting partway through a video, enter the current video position in seconds before creating the session. Start listening, then play YouTube or another browser video. The browser companion is not required for this feature.

On macOS, Video sessions use **Mac system audio**, including playback through headphones, and require Screen & System Audio Recording permission. On Windows/Linux, route browser playback to a virtual/loopback audio input and explicitly select that device. NextCue does not silently use the microphone for a Video session. System capture includes other apps' audio too, so keep the intended video as the only sound source.

Choose **Notes** to refresh study notes approximately once a minute while new speech arrives, or create/update notes manually. Notes use the selected local or connected account provider and only the captured transcript. **Subtitles** captures timed speech without requiring an answer provider, and disables automatic question answering. Set up local transcription once in Settings for either mode. Both modes retain the transcript and can produce subtitles; switching modes does not discard captured speech.

Captions appear in NextCue and its floating assistant, with a delay from Whisper's short audio segments and inference time. Choose **Export SRT** or **Export VTT** to save recognized speech with actual Whisper segment start/end times; exporting pauses capture and drains the final audio before opening the save dialog. Saved notes and transcript can be exported with the session's Markdown export.

Subtitle timing follows elapsed capture time plus the starting video position you entered. NextCue does not read the browser player's playhead or automatically follow pauses, seeks, or playback-speed changes. For an aligned export, play continuously at normal speed and start listening alongside playback. Start a new Video session with the new position after seeking or changing speed. Previously saved transcripts without speech timings remain available for notes and Markdown export, but do not acquire invented subtitle durations. These captions describe captured speech; they are not burned into a video file or inserted into YouTube's player.

## Floating assistant

For Zoom or other full-screen sharing, choose **Take notes in background** in an open session. NextCue starts note capture and hides the full window and floating pin only after listening starts successfully. Notes continue while you use the meeting, browser, or editor. **Hide for screen sharing** hides the current assistant without changing the session's assistance mode. Bring it back through the tray menu or the hide/show shortcut. A visible NextCue window can still appear in a Mac full-screen share; hiding the window is the supported background workflow.

Open a session and choose **Pin mode** to shrink NextCue to a movable assistant button. Drag its small grip to place it beside your call. Click the button to reveal the latest answer, or generate an answer to the latest unanswered question. **Get answer** can generate another suggestion; the microphone button starts or pauses listening. **Open full app** restores your workspace.

The pin and full workspace share the same live session: switching modes preserves listening, streamed answers, and saved history. Transcript and background are available as optional side panels in the full workspace. Add personal context during session creation under **Personalize answers**, or edit it later.

Enable **Allow clicks through floating assistant** in **Settings → Assistant & screen sharing** to let mouse clicks reach the application beneath the pin or answer card. Floating windows do not request keyboard focus; the full workspace restores normal input. With clicks passing through, use the answer shortcut to reveal/collapse the card. Hide and show it with the visibility shortcut to recover the full workspace, or use the tray's **Show NextCue**. This setting changes input behavior, not screen-capture visibility. Hidden/floating windows also remove their running Dock/taskbar presence where supported; the tray remains available for recovery.

Use **Cmd+Shift+Space** on macOS or **Ctrl+Shift+Space** on Windows/Linux to show or collapse the answer card when the operating system allows shortcut registration. **Cmd/Ctrl+Shift+H** hides or shows the assistant without stopping listening or answer generation. Edit these bindings in **Settings → Assistant & screen sharing**, then choose **Save shortcuts**. Shortcut text uses combinations such as `CommandOrControl+Alt+A`; invalid or conflicting registrations do not replace your saved bindings. The default hide/show binding remains available as a recovery shortcut when registration succeeds. The pin position is remembered locally and expanded answers are kept within the display's work area.

Choose **Exclude assistant from screen capture when supported** in Settings to request Windows capture exclusion, reported only as **best effort**. Turn it off to include the visible assistant in a recording. macOS/Linux report **May appear in screen sharing** while exclusion is enabled because they cannot reliably exclude the window. **Hide assistant** hides the native window and removes its interface content from the webview while session work continues. The hide/show shortcut brings it back. Hiding removes the window from the visible desktop; it cannot erase earlier footage, control a recorder's cached frames, or guarantee exclusion from every recording method. These controls do not stop a recording or a screen share, and do not hide the system pointer. Verify the actual recording/share preview. Sharing a specific application window or using an unshared monitor can keep the assistant separate when those options are available. Pin/focus/shortcut behavior needs native verification on Windows/Linux; Linux window-manager restrictions can limit placement.

In the focused full app, **Tab / Shift+Tab** moves between controls and **Enter / Space** activates buttons. Focus the answer region to scroll with arrow keys or **Page Up / Page Down**. **Cmd/Ctrl+Enter** answers the current question only on the live Answer tab, with no dialog open. There is no concealed mouse or recorder-wide cursor control; keyboard navigation does not change the recorder's capture settings.

## System tray and meeting triggers

NextCue keeps a system tray menu available while the app is running. With **Settings → Background & meetings → Keep running in system tray** enabled, closing the window hides it and keeps the current session running. Use **Quit NextCue** in the tray to save and exit. If the desktop environment cannot provide a tray, closing falls back to the normal save-and-quit flow. **Start in system tray** starts hidden when you launch the app; it does not register an OS login item and never starts audio by itself.

The tray provides **Show NextCue**, **Hide NextCue**, **Run meeting trigger**, **Start listening**, **Take meeting notes**, **Answer questions**, **Pause listening**, **Settings**, and **Quit NextCue**. The tray status shows whether listening is active and which assistance mode is selected. Actions reuse the current unfinished session, or create a Meeting session when none is open. Starting notes while not listening creates a separate Meeting session if an Interview or Practice session is selected. Switching to notes during active listening keeps the current transcript and uses meeting-note instructions. Notes mode suppresses automatic question answers and refreshes notes about once a minute when new transcript arrives. Pausing from the tray requests a final notes update when needed. Answer mode enables automatic suggestions for detected questions. Both use the audio input and AI provider chosen in Settings; transcription and provider setup must be ready.

Choose **When a meeting is triggered**: **Do nothing** (the default), **Ask me first**, **Listen and take notes**, or **Listen and answer**. The customizable **Start meeting shortcut** defaults to **Cmd/Ctrl+Shift+M** and uses this preference. Explicit tray actions work regardless of the configured automatic action. Showing the app or opening Settings from the tray preserves background listening and generation. Listening continues until paused; hints do not detect the end of a call.

**Detect meeting hints** is off by default. When enabled, executable-name inspection can hint that Zoom or Microsoft Teams opened on macOS/Windows, or Zoom opened on Linux. The first process sample establishes a baseline, so an app already open at launch does not automatically trigger recording. Process presence does not prove that a meeting is active. It does not inspect window titles, screen contents, audio, or browser history.

For Google Meet and browser meetings, pair the optional bundled browser companion. Expand **Google Meet and browser meetings**, choose **Open companion folder**, and follow its [setup instructions](browser-extension/README.md). **Connect browser companion → Copy connection code** provides a private local pairing code to paste into the extension's options. The Chromium/Edge companion supports Google Meet, Zoom web, Teams web, Webex, Jitsi at meet.jit.si, and Whereby meeting routes. A Firefox development version is also included. Other sites and apps can use manual tray triggers.

The companion sends only a provider label and a random page ID to an authenticated `127.0.0.1` listener. It does not send meeting URLs, call codes, page contents, browser credentials, or transcripts. A page-open hint does not confirm that you joined a call. The desktop pairing token is kept in the OS credential store, the browser copy stays in local extension storage, and neither is stored with session settings. The listener starts only after detection is enabled or connection setup is explicitly requested. Disable detection to stop automatic actions. Browser installation and pairing are explicit user steps; this build does not alter browser settings automatically. Live provider calls and Windows/Linux behavior still need verification on those systems.

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

With local AI and Jev disabled, prompts and inference remain on this computer. The app accepts only loopback Ollama addresses and rejects cloud model tags and remote-backed aliases. Choosing ChatGPT or Claude sends the question, supplied background, session metadata, and relevant transcript to that provider. Enabling Jev sends the same context to TypeSafe for a structured check. Speech transcription stays local for every provider. Listening starts through an explicit app/tray action or a meeting action the user enabled in Settings; meeting detection and automatic meeting actions are off by default. The initial Whisper download requires internet access. Audio segments are held in bounded memory queues and are not saved as recordings.

Settings and sessions are written atomically to the app data directory, normally `~/Library/Application Support/com.nextcue.desktop` on macOS. Whisper weights are in its `models` subdirectory. Transcript timestamps use the actual audio/question time. Local records are plain JSON; session exports are Markdown and subtitle exports are UTF-8 SRT/WebVTT. ChatGPT tokens and the optional TypeSafe key use the OS credential store. Claude Code owns its own authentication. Provider account access must be completed by the user; development verification does not log into accounts or consume subscription allowance.

For isolated QA, set `NEXTCUE_DATA_DIR` to an absolute directory before launching. This redirects settings and model storage. It does not change the user's Ollama model library.

## Verification

```sh
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

The UI tests exercise persistence, streaming cancellation across sessions, preserving the final audio segment after stopping, truthful browser-preview behavior, and companion mode continuity for listening, answers, shortcuts, and recaps. Video coverage checks notes, provider-free subtitles, system-audio selection, and subtitle export with preserved timings and final speech. They inject a test-only desktop bridge. Rust tests exercise the actual service parsing, local-only restrictions, storage, native audio processing, subtitle formats and cue timing, and companion placement. Start a separate UI server and use installed Chrome:

```sh
npm run dev:web
npm run test:ui
```

Ignored integration tests can run against real local Ollama and a local Whisper model; see the tests in `src-tauri/src/ollama.rs` and `src-tauri/src/voice.rs` for their environment variables. The browser preview is for interface development; Ollama, native audio, file dialogs, and local app storage run in the desktop application.

## Current limits

Whisper processes short audio segments, so transcript/answer latency depends on the model and hardware. There is no speaker diarization or verified active-call detection: optional Zoom/Teams executable hints and browser meeting-page hints describe presence, rather than proving that you joined a call. Choose Meeting mode for grounded notes. PDF/DOCX resume parsing, screen/coding-image analysis, billing, and public app distribution are outside this first version. AI suggestions can be incorrect; review them and use your own experience. This app does not claim to be hidden from screen sharing. Live ChatGPT/Claude/Jev inference requires the user's connected account/key and has not been verified with a paid account in this workspace.

Primary references: [Ollama chat API](https://docs.ollama.com/api/chat), [Tauri](https://v2.tauri.app/), [whisper-rs](https://docs.rs/whisper-rs/0.15.1/whisper_rs/), [ScreenCaptureKit Rust bindings](https://docs.rs/screencapturekit/11.0.0/screencapturekit/), [WebVTT specification](https://www.w3.org/TR/webvtt1/), [SRT format description](https://www.loc.gov/preservation/digital/formats/fdd/fdd000569.shtml).
