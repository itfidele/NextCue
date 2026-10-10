export type SessionMode = "Interview" | "Practice" | "Meeting";
export type SessionStatus = "ready" | "active" | "ended";
export type AnswerProvider = "local" | "chatgpt" | "claude";
export interface TranscriptEntry {
  id: string;
  text: string;
  time: number;
  source: "microphone" | "system" | "typed";
}
export interface Answer {
  id: string;
  question: string;
  text: string;
  time: number;
  model: string;
  durationMs?: number;
}
export interface InterviewSession {
  id: string;
  title: string;
  company: string;
  role: string;
  mode: SessionMode;
  createdAt: number;
  status: SessionStatus;
  duration: number;
  context: string;
  instructions: string;
  transcript: TranscriptEntry[];
  answers: Answer[];
  summary?: string;
}
export interface Settings {
  model: string;
  ollamaUrl: string;
  language: string;
  autoAnswer: boolean;
  audioSource: "microphone" | "system";
  inputDevice: string;
  captureProtection?: boolean;
  answerShortcut?: string;
  visibilityShortcut?: string;
  answerProvider?: AnswerProvider;
  chatgptModel?: string;
  jevEnabled?: boolean;
}
export interface AppState {
  sessions: InterviewSession[];
  settings: Settings;
}
export interface RuntimeStatus {
  ollama: boolean;
  models: { name: string; size: number }[];
  transcription: boolean;
  transcriptionError?: string;
  error?: string;
  platform: string;
}
export interface ChatRequest {
  requestId: string;
  model: string;
  question: string;
  context: string;
  instructions: string;
  transcript: string;
  kind?: "answer" | "summary";
  provider?: AnswerProvider;
  sessionMode?: SessionMode;
  sessionTitle?: string;
  role?: string;
  company?: string;
}
export interface ChatgptStatus {
  connected: boolean;
  email?: string;
  planUsage: boolean;
  models: { slug: string; displayName: string }[];
  accounts?: {
    id: string;
    label: string;
    email?: string;
    connected: boolean;
  }[];
  activeAccountId?: string;
  error?: string;
}
export interface ClaudeStatus {
  available: boolean;
  connected: boolean;
  subscription: boolean;
  email?: string;
  detail?: string;
}
export interface ContextAssessment {
  decision: "answer" | "clarify" | "notes" | "wait";
  confidence: number;
  backgroundSupported: number;
}
export interface ChatResult {
  text: string;
  model: string;
  durationMs: number;
}
export interface TokenEvent {
  requestId: string;
  token: string;
}
export interface VoiceEvent {
  sessionId: string;
  text: string;
  source: "microphone" | "system";
  time: number;
}
export interface VoiceProgress {
  progress: number;
  status: string;
}
export interface InputDevice {
  id: string;
  name: string;
}
export type CompanionMode = "main" | "pin" | "answer";
export interface CompanionConfiguration {
  captureProtection: boolean;
  answerShortcut: string;
  visibilityShortcut: string;
}
export interface CompanionState {
  mode: CompanionMode;
  captureProtection: "best-effort" | "unavailable" | "disabled";
  shortcutAvailable?: boolean;
  visibilityShortcutAvailable?: boolean;
  emergencyShortcutAvailable?: boolean;
  visible?: boolean;
  answerShortcut?: string;
  visibilityShortcut?: string;
  captureProtectionEnabled?: boolean;
}
export interface DesktopAPI {
  getState(): Promise<AppState>;
  saveState(state: AppState): Promise<void>;
  status(): Promise<RuntimeStatus>;
  startOllama(): Promise<void>;
  chat(request: ChatRequest): Promise<ChatResult>;
  cancelChat(requestId: string): Promise<void>;
  getChatgptStatus?(refreshModels?: boolean): Promise<ChatgptStatus>;
  signInChatgpt?(accountId?: string): Promise<ChatgptStatus>;
  cancelChatgptSignIn?(): Promise<void>;
  signOutChatgpt?(): Promise<ChatgptStatus>;
  selectChatgptAccount?(accountId: string): Promise<ChatgptStatus>;
  getClaudeStatus?(): Promise<ClaudeStatus>;
  openProviderPage?(page: "chatgpt-usage" | "claude-setup"): Promise<void>;
  getJevStatus?(): Promise<{ configured: boolean }>;
  saveJevKey?(key: string): Promise<{ configured: boolean }>;
  deleteJevKey?(): Promise<{ configured: boolean }>;
  assessContext?(request: ChatRequest): Promise<ContextAssessment>;
  onToken(callback: (event: TokenEvent) => void): () => void;
  inputDevices(): Promise<InputDevice[]>;
  startListening(request: {
    device: string;
    language: string;
    sessionId: string;
    source: "microphone" | "system";
  }): Promise<void>;
  stopListening(): Promise<void>;
  onTranscript(callback: (event: VoiceEvent) => void): () => void;
  onVoiceError(
    callback: (event: { sessionId: string; error: string }) => void,
  ): () => void;
  onVoiceStatus(
    callback: (event: { sessionId: string; listening: boolean }) => void,
  ): () => void;
  onVoiceProgress(callback: (event: VoiceProgress) => void): () => void;
  setupVoice(): Promise<void>;
  exportSession(session: InterviewSession): Promise<boolean>;
  importContext(): Promise<{ name: string; text: string } | null>;
  copyText?(text: string): Promise<void>;
  startDragging?(): Promise<void>;
  setAlwaysOnTop(value: boolean): Promise<void>;
  setCompanionMode?(mode: CompanionMode): Promise<CompanionState>;
  onCompanionToggle?(callback: () => void): () => void;
  configureCompanion?(
    configuration: CompanionConfiguration,
  ): Promise<CompanionState>;
  getCompanionStatus?(): Promise<CompanionState>;
  setCompanionVisible?(visible: boolean): Promise<CompanionState>;
  onCompanionVisibility?(callback: (visible: boolean) => void): () => void;
  minimize(): void;
  onBeforeQuit(callback: () => void): () => void;
  quitApp(state: AppState | null): Promise<void>;
}
declare global {
  interface Window {
    desktop?: DesktopAPI;
  }
}
