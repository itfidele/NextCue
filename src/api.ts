import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppState, BackgroundAction, DesktopAPI } from "./types";

export const defaults: AppState = {
  sessions: [],
  settings: {
    model: "qwen2.5:7b-instruct",
    ollamaUrl: "http://127.0.0.1:11434",
    language: "en",
    autoAnswer: true,
    audioSource: "microphone",
    inputDevice: "",
    captureProtection: true,
    clickThrough: false,
    answerShortcut: "CommandOrControl+Shift+Space",
    visibilityShortcut: "CommandOrControl+Shift+H",
    answerProvider: "local",
    chatgptModel: "",
    jevEnabled: false,
    runInBackground: true,
    startHidden: false,
    meetingDetection: false,
    meetingAction: "none",
    meetingShortcut: "CommandOrControl+Shift+M",
  },
};
function subscribe<T>(
  event: string,
  callback: (payload: T) => void,
  onReady?: (promise: Promise<void>) => void,
): () => void {
  let disposed = false;
  let unsubscribe: (() => void) | undefined;
  const ready = listen<T>(event, (e) => {
    if (!disposed) callback(e.payload);
  }).then((fn) => {
    if (disposed) fn();
    else unsubscribe = fn;
  });
  onReady?.(ready);
  return () => {
    disposed = true;
    unsubscribe?.();
  };
}
export const native = isTauri();
let backgroundSubscribed: Promise<void> = Promise.resolve();
let meetingHintSubscribed: Promise<void> = Promise.resolve();
const desktop: DesktopAPI = {
  getState: () => invoke("get_state"),
  saveState: (state) => invoke("save_state", { state }),
  status: () => invoke("status"),
  startOllama: () => invoke("start_ollama"),
  chat: (request) => invoke("chat", { request }),
  cancelChat: (requestId) => invoke("cancel_chat", { requestId }),
  getChatgptStatus: (refreshModels = false) =>
    invoke("chatgpt_status", { refreshModels }),
  signInChatgpt: (accountId) => invoke("chatgpt_sign_in", { accountId }),
  cancelChatgptSignIn: () => invoke("chatgpt_cancel_sign_in"),
  signOutChatgpt: () => invoke("chatgpt_sign_out"),
  selectChatgptAccount: (accountId) =>
    invoke("chatgpt_select_account", { accountId }),
  getClaudeStatus: () => invoke("claude_status"),
  openProviderPage: (page) => invoke("open_provider_page", { page }),
  getJevStatus: () => invoke("jev_status"),
  saveJevKey: (key) => invoke("jev_save_key", { key }),
  deleteJevKey: () => invoke("jev_delete_key"),
  assessContext: (request) => invoke("jev_assess", { request }),
  onToken: (callback) => subscribe("ollama-token", callback),
  inputDevices: () => invoke("input_devices"),
  startListening: (request) => invoke("start_listening", { request }),
  stopListening: () => invoke("stop_listening"),
  onTranscript: (callback) => subscribe("transcript", callback),
  onVoiceError: (callback) => subscribe("voice-error", callback),
  onVoiceStatus: (callback) => subscribe("voice-status", callback),
  onVoiceProgress: (callback) => subscribe("voice-progress", callback),
  setupVoice: () => invoke("setup_voice"),
  exportSession: (session) => invoke("export_session", { session }),
  exportSubtitles: (session, format) => invoke("export_subtitles", { session, format }),
  importContext: () => invoke("import_context"),
  copyText: (text) => invoke("copy_text", { text }),
  startDragging: () => getCurrentWindow().startDragging(),
  setAlwaysOnTop: (value) => getCurrentWindow().setAlwaysOnTop(value),
  setCompanionMode: (mode) => invoke("set_companion_mode", { mode }),
  onCompanionToggle: (callback) => subscribe("companion-toggle", callback),
  configureCompanion: (configuration) =>
    invoke("configure_companion", { ...configuration }),
  getCompanionStatus: () => invoke("get_companion_status"),
  setCompanionVisible: (visible) =>
    invoke("set_companion_visible", { visible }),
  onCompanionVisibility: (callback) =>
    subscribe<{ visible: boolean }>("companion-visibility", (event) =>
      callback(event.visible),
    ),
  configureBackground: (config) => invoke("configure_background", { config }),
  getBackgroundStatus: () => invoke("get_background_status"),
  onBackgroundAction: (callback) => {
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    backgroundSubscribed = listen<BackgroundAction>("background-action", (event) => {
      if (!disposed) callback(event.payload);
    }).then((stop) => {
      if (disposed) stop();
      else unsubscribe = stop;
    });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  },
  onMeetingHint: (callback) => subscribe("meeting-hint", callback, (ready) => { meetingHintSubscribed = ready; }),
  backgroundReady: async () => {
    await Promise.all([backgroundSubscribed, meetingHintSubscribed]);
    await invoke("background_ready");
  },
  getBrowserConnection: () => invoke("get_browser_connection"),
  openBrowserCompanion: () => invoke("open_browser_companion"),
  updateBackgroundActivity: (activity) => invoke("update_background_activity", { ...activity }),
  minimize: () => {
    void getCurrentWindow().minimize();
  },
  onBeforeQuit: (callback) => subscribe("before-quit", callback),
  quitApp: (state) => invoke("quit_app", { state }),
};
const desktopOnly = async (): Promise<never> => {
  throw new Error(
    "Open the NextCue desktop app to use your local models and audio.",
  );
};
// The web view is a UI preview. It never fabricates AI answers or captures audio.
const preview: DesktopAPI = {
  getState: async () => {
    try {
      return (
        JSON.parse(localStorage.getItem("nextcue-preview") || "null") ||
        defaults
      );
    } catch {
      return defaults;
    }
  },
  saveState: async (state) => {
    localStorage.setItem("nextcue-preview", JSON.stringify(state));
  },
  status: async () => ({
    ollama: false,
    models: [],
    transcription: false,
    error: "Desktop preview",
    platform: "web",
  }),
  startOllama: desktopOnly,
  chat: desktopOnly,
  cancelChat: async () => {},
  getChatgptStatus: async () => ({
    connected: false,
    planUsage: false,
    models: [],
  }),
  signInChatgpt: desktopOnly,
  cancelChatgptSignIn: async () => {},
  signOutChatgpt: desktopOnly,
  selectChatgptAccount: desktopOnly,
  getClaudeStatus: async () => ({
    available: false,
    connected: false,
    subscription: false,
  }),
  openProviderPage: desktopOnly,
  getJevStatus: async () => ({ configured: false }),
  saveJevKey: desktopOnly,
  deleteJevKey: desktopOnly,
  assessContext: desktopOnly,
  onToken: () => () => {},
  inputDevices: async () => [],
  startListening: desktopOnly,
  stopListening: async () => {},
  onTranscript: () => () => {},
  onVoiceError: () => () => {},
  onVoiceStatus: () => () => {},
  onVoiceProgress: () => () => {},
  setupVoice: desktopOnly,
  exportSession: desktopOnly,
  exportSubtitles: desktopOnly,
  importContext: desktopOnly,
  copyText: (text) => navigator.clipboard.writeText(text),
  setAlwaysOnTop: desktopOnly,
  setCompanionMode: desktopOnly,
  onCompanionToggle: () => () => {},
  configureCompanion: desktopOnly,
  getCompanionStatus: desktopOnly,
  setCompanionVisible: desktopOnly,
  onCompanionVisibility: () => () => {},
  configureBackground: desktopOnly,
  getBackgroundStatus: desktopOnly,
  onBackgroundAction: () => () => {},
  onMeetingHint: () => () => {},
  backgroundReady: async () => {},
  getBrowserConnection: desktopOnly,
  openBrowserCompanion: desktopOnly,
  updateBackgroundActivity: async () => {},
  minimize: () => {},
  onBeforeQuit: () => () => {},
  quitApp: desktopOnly,
};
export const api = window.desktop || (native ? desktop : preview);
