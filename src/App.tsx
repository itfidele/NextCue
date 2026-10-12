import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ArrowDownToLine,
  ArrowLeft,
  ArrowRight,
  AudioLines,
  Bell,
  BookOpen,
  Check,
  CheckCircle2,
  ChevronDown,
  Clock3,
  Copy,
  Cpu,
  EyeOff,
  FileText,
  FolderOpen,
  History,
  Home,
  GripHorizontal,
  Maximize2,
  MessageCircle,
  Loader2,
  Mic,
  MoreHorizontal,
  Pause,
  Pin,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Monitor,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  Upload,
  Video,
  Captions,
  X,
  Zap,
} from "lucide-react";
import Markdown from "react-markdown";
import { api, defaults, native } from "./api";
import type {
  AppState,
  InterviewSession,
  RuntimeStatus,
  SessionMode,
  Settings,
  InputDevice,
  CompanionMode,
  CompanionState,
  CompanionConfiguration,
  AnswerProvider,
  ChatgptStatus,
  ClaudeStatus,
  ChatRequest,
  AssistanceMode,
  BackgroundAction,
  BackgroundConfiguration,
  BackgroundStatus,
  BrowserConnection,
  MeetingHint,
} from "./types";

const uid = () => crypto.randomUUID();
const duration = (seconds: number) =>
  `${Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
const date = (time: number) =>
  new Date(time).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
const time = (value: number) =>
  new Date(value).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
const byteLength = (text: string) => new TextEncoder().encode(text).length;
function sessionInputError(
  fields: Partial<
    Pick<
      InterviewSession,
      "title" | "role" | "company" | "context" | "instructions"
    >
  >,
) {
  for (const [key, label, limit] of [
    ["title", "the session title", 256],
    ["role", "the role or objective", 256],
    ["company", "the company name", 256],
    ["context", "your background", 100000],
    ["instructions", "the answer guidance", 20000],
  ] as const) {
    if (fields[key] && byteLength(fields[key]!) > limit)
      return `Shorten ${label} before saving.`;
  }
  return "";
}
const notesPrompt = (session?: InterviewSession) =>
  session?.mode === "Video" ? "Create video notes from the captured transcript." : "Create meeting notes.";
const mediaTime = (session: InterviewSession, epochMs: number) => {
  const seconds = Math.max(0, Math.floor((epochMs - (session.mediaStartedAt ?? epochMs)) / 1000 + (session.mediaOffsetSeconds ?? 0)));
  return `${Math.floor(seconds / 3600).toString().padStart(2, "0")}:${duration(seconds % 3600)}`;
};

function Modal({
  children,
  title,
  subtitle,
  onClose,
  wide = false,
}: {
  children: ReactNode;
  title: string;
  subtitle?: string;
  onClose: () => void;
  wide?: boolean;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement;
    (
      panel.current?.querySelector<HTMLElement>("input, textarea, select") ||
      panel.current?.querySelector<HTMLElement>("button")
    )?.focus();
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeRef.current();
      if (e.key !== "Tab") return;
      const items = panel.current?.querySelectorAll<HTMLElement>(
        "button:not(:disabled), input, textarea, select, a[href]",
      );
      if (!items?.length) return;
      const first = items[0],
        last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      }
      if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handler);
    return () => {
      document.removeEventListener("keydown", handler);
      previous?.focus();
    };
  }, []);
  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className={`modal ${wide ? "wide" : ""}`}
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={title}
      >
        <button
          className="icon-button close-modal"
          aria-label="Close dialog"
          onClick={onClose}
        >
          <X size={19} />
        </button>
        <h2>{title}</h2>
        {subtitle && <p className="muted modal-subtitle">{subtitle}</p>}
        {children}
      </div>
    </div>
  );
}

export default function App() {
  const [state, setState] = useState<AppState>(defaults);
  const [loaded, setLoaded] = useState(false);
  const [initialChecksComplete, setInitialChecksComplete] = useState(false);
  const [runtime, setRuntime] = useState<RuntimeStatus>({
    ollama: false,
    models: [],
    transcription: false,
    platform: "",
  });
  const [view, setView] = useState<
    "workspace" | "history" | "settings" | "live"
  >("workspace");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [modal, setModal] = useState<
    "create" | "help" | "context" | "delete" | null
  >(null);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState("all");
  const [toast, setToast] = useState("");
  const [checking, setChecking] = useState(false);
  const [chatgpt, setChatgpt] = useState<ChatgptStatus>({
    connected: false,
    planUsage: false,
    models: [],
  });
  const [claude, setClaude] = useState<ClaudeStatus>({
    available: false,
    connected: false,
    subscription: false,
  });
  const [providerBusy, setProviderBusy] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const [jevConfigured, setJevConfigured] = useState(false);
  const [jevKey, setJevKey] = useState("");
  const [jevBusy, setJevBusy] = useState(false);
  const [jevError, setJevError] = useState("");
  const [contextInsight, setContextInsight] = useState("");
  const [providerError, setProviderError] = useState("");
  const [dismissedMeetingHint, setDismissedMeetingHint] = useState<
    string | null
  >(null);
  const [listening, setListening] = useState(false);
  const [audioBusy, setAudioBusy] = useState(false);
  const audioOperationRef = useRef(false);
  const [voiceSetup, setVoiceSetup] = useState(false);
  const [voiceProgress, setVoiceProgress] = useState({
    progress: 0,
    status: "",
  });
  const [devices, setDevices] = useState<InputDevice[]>([]);
  const [videoInputs, setVideoInputs] = useState<Record<string, string>>({});
  const [question, setQuestion] = useState("");
  const [generating, setGenerating] = useState(false);
  const [streamText, setStreamText] = useState("");
  const [answerIndex, setAnswerIndex] = useState<number | null>(null);
  const [companion, setCompanion] = useState<CompanionState>({
    mode: "main",
    captureProtection: "unavailable",
  });
  const [companionBusy, setCompanionBusy] = useState(false);
  const [backgroundNotesBusy, setBackgroundNotesBusy] = useState(false);
  const backgroundNotesBusyRef = useRef(false);
  const [shortcutDraft, setShortcutDraft] = useState({
    answerShortcut: defaults.settings.answerShortcut!,
    visibilityShortcut: defaults.settings.visibilityShortcut!,
    meetingShortcut: defaults.settings.meetingShortcut!,
  });
  const [shortcutSaving, setShortcutSaving] = useState(false);
  const [shortcutError, setShortcutError] = useState("");
  const [background, setBackground] = useState<BackgroundStatus>({
    runInBackground: true,
    startHidden: false,
    meetingDetection: false,
    meetingAction: "none",
    trayAvailable: false,
    detectionAvailable: false,
  });
  const [backgroundSaving, setBackgroundSaving] = useState(false);
  const [backgroundError, setBackgroundError] = useState("");
  const [meetingHint, setMeetingHint] = useState<MeetingHint | null>(null);
  const [browserConnection, setBrowserConnection] = useState<BrowserConnection | null>(null);
  const [browserBusy, setBrowserBusy] = useState(false);
  const backgroundSavingRef = useRef(false);
  const backgroundQueueRef = useRef<Promise<void>>(Promise.resolve());
  const backgroundHandlerRef = useRef<(event: BackgroundAction) => Promise<void>>(async () => {});
  const meetingHintHandlerRef = useRef<(event: MeetingHint) => Promise<void>>(async () => {});
  const pendingMeetingHintRef = useRef<MeetingHint | null>(null);
  const notesWatermarkRef = useRef<Record<string, number>>({});
  const companionBusyRef = useRef(false);
  const companionVisibilityRevisionRef = useRef(0);
  const [detailsTab, setDetailsTab] = useState<"transcript" | "context" | null>(
    null,
  );
  const [answerTab, setAnswerTab] = useState<"answer" | "summary">("answer");
  const [draft, setDraft] = useState({
    title: "",
    company: "",
    role: "",
    mode: "Interview" as SessionMode,
    context: "",
    instructions: "",
    mediaOffsetSeconds: "0",
  });
  const stateRef = useRef(state);
  stateRef.current = state;
  const selectedRef = useRef(selectedId);
  selectedRef.current = selectedId;
  const listeningRef = useRef(false);
  const acceptingTranscriptRef = useRef<string | null>(null);
  const pendingAutoQuestion = useRef<{
    sessionId: string;
    text: string;
  } | null>(null);
  const quitting = useRef(false);
  const loadedRef = useRef(loaded);
  loadedRef.current = loaded;
  const requestRef = useRef<{ id: string; sessionId: string } | null>(null);
  const requestCompletionRef = useRef<{ id: string; promise: Promise<void> } | null>(null);
  const autoTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const transcriptEnd = useRef<HTMLDivElement>(null);
  const initialized = useRef(false);
  const saveErrorShown = useRef(false);
  const selected = state.sessions.find((s) => s.id === selectedId);
  const notify = (message: string) => {
    setToast(message);
  };
  const updateSession = (
    id: string,
    update: (s: InterviewSession) => InterviewSession,
  ) =>
    setState((s) => ({
      ...s,
      sessions: s.sessions.map((item) =>
        item.id === id ? update(item) : item,
      ),
    }));
  const updateSettings = (patch: Partial<Settings>) =>
    setState((s) => ({ ...s, settings: { ...s.settings, ...patch } }));
  const autoAnswersEnabled = (sessionId: string) => {
    const session = stateRef.current.sessions.find((s) => s.id === sessionId);
    if (session?.mode === "Video" || session?.assistanceMode === "subtitles") return false;
    return session?.assistanceMode === "answers" ||
      (session?.assistanceMode !== "notes" && session?.assistanceMode !== "manual" &&
        stateRef.current.settings.autoAnswer);
  };

  function acceptChatgpt(result: ChatgptStatus) {
    setChatgpt(result);
    if (
      result.models.length &&
      !result.models.some(
        (m) => m.slug === stateRef.current.settings.chatgptModel,
      )
    ) {
      updateSettings({ chatgptModel: result.models[0].slug });
    }
  }
  async function refreshProviders(refreshModels = false) {
    const results = await Promise.allSettled([
      api.getChatgptStatus?.(
        refreshModels && stateRef.current.settings.answerProvider === "chatgpt",
      ),
      api.getClaudeStatus?.(),
    ]);
    if (results[0].status === "fulfilled" && results[0].value)
      acceptChatgpt(results[0].value);
    else if (results[0].status === "rejected") {
      const error = errorText(results[0].reason);
      setChatgpt((previous) => ({
        ...previous,
        connected: false,
        planUsage: false,
        error,
      }));
    }
    if (results[1].status === "fulfilled" && results[1].value)
      setClaude(results[1].value);
    else if (results[1].status === "rejected") {
      const detail = errorText(results[1].reason);
      setClaude((previous) => ({
        ...previous,
        connected: false,
        subscription: false,
        detail,
      }));
    }
  }
  async function connectChatgpt(accountId?: string) {
    if (!api.signInChatgpt || signingIn) return;
    setSigningIn(true);
    setProviderError("");
    try {
      const result = await api.signInChatgpt(accountId);
      acceptChatgpt(result);
      if (result.error) setProviderError(result.error);
    } catch (error) {
      setProviderError(errorText(error));
    } finally {
      setSigningIn(false);
    }
  }
  async function checkProviderConnection() {
    setProviderBusy(true);
    setProviderError("");
    try {
      await refreshProviders(true);
    } finally {
      setProviderBusy(false);
    }
  }
  async function saveContextHelper() {
    if (!api.saveJevKey || !jevKey.trim()) return;
    setJevBusy(true);
    setJevError("");
    try {
      const result = await api.saveJevKey(jevKey.trim());
      setJevConfigured(result.configured);
      updateSettings({ jevEnabled: result.configured });
    } catch (error) {
      setJevError(errorText(error));
    } finally {
      setJevKey("");
      setJevBusy(false);
    }
  }
  useEffect(() => {
    if (
      chatgpt.models.length &&
      !chatgpt.models.some(
        (model) => model.slug === state.settings.chatgptModel,
      )
    ) {
      updateSettings({ chatgptModel: chatgpt.models[0].slug });
    }
  }, [chatgpt.models, state.settings.chatgptModel]);

  const refresh = async () => {
    setChecking(true);
    try {
      const result = await api.status();
      setRuntime(result);
    } catch (e) {
      setRuntime((r) => ({ ...r, ollama: false, error: errorText(e) }));
    } finally {
      setChecking(false);
    }
  };
  useEffect(() => {
    if (!initialized.current) {
      initialized.current = true;
      api
        .getState()
        .then((result) => {
          setState({
            ...defaults,
            ...result,
            settings: { ...defaults.settings, ...result.settings },
          });
          setLoaded(true);
        })
        .catch((e) => notify(`Could not load local data: ${errorText(e)}`));
      void Promise.all([refresh(), refreshProviders()]).then(() =>
        setInitialChecksComplete(true),
      );
      api
        .getJevStatus?.()
        .then((result) => setJevConfigured(result.configured))
        .catch(() => {});
      api
        .inputDevices()
        .then(setDevices)
        .catch(() => {});
    }
    const timer = setInterval(() => void refresh(), 15000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      api.saveState(state).catch((e) => {
        if (!saveErrorShown.current) {
          notify(`Changes could not be saved: ${errorText(e)}`);
          saveErrorShown.current = true;
        }
      });
    }, 500);
    return () => clearTimeout(timer);
  }, [state, loaded]);
  useEffect(() => {
    if (toast) {
      const t = setTimeout(() => setToast(""), 6000);
      return () => clearTimeout(t);
    }
  }, [toast]);
  useEffect(() => {
    if (!selected || selected.status !== "active") return;
    const timer = setInterval(
      () =>
        updateSession(selected.id, (s) => ({ ...s, duration: s.duration + 1 })),
      1000,
    );
    return () => clearInterval(timer);
  }, [selected?.id, selected?.status]);
  useEffect(() => {
    transcriptEnd.current?.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
    });
  }, [selected?.transcript.length]);
  useEffect(() => {
    setContextInsight("");
  }, [selectedId]);

  async function generate(q: string, kind: "answer" | "summary" = "answer") {
    const session = stateRef.current.sessions.find(
      (s) => s.id === selectedRef.current,
    );
    if (!session || requestRef.current || (!q.trim() && kind === "answer"))
      return;
    if (kind === "answer" && (session.mode === "Video" || session.assistanceMode === "subtitles")) return;
    if (kind === "answer" && byteLength(q.trim()) > 16000) {
      notify("Shorten the question before requesting an answer.");
      return;
    }
    if (providerBusy || signingIn) {
      notify("Finish the account connection before requesting an answer.");
      return;
    }
    const provider = stateRef.current.settings.answerProvider || "local";
    if (provider === "chatgpt" && (!chatgpt.connected || !chatgpt.planUsage)) {
      notify("Connect ChatGPT and enable plan usage in Settings first.");
      return;
    }
    if (
      provider === "chatgpt" &&
      !chatgpt.models.some(
        (m) => m.slug === stateRef.current.settings.chatgptModel,
      )
    ) {
      notify("Refresh your ChatGPT models in Settings and choose one.");
      return;
    }
    if (provider === "claude" && (!claude.connected || !claude.subscription)) {
      notify(
        "Sign into Claude Code with your Claude account, then check the connection in Settings.",
      );
      return;
    }
    if (provider === "local" && !runtime.ollama) {
      notify("Start Ollama in Settings before generating an answer.");
      return;
    }
    if (
      provider === "local" &&
      !runtime.models.some((m) => m.name === stateRef.current.settings.model)
    ) {
      notify("Choose an installed local model in Settings.");
      return;
    }
    if (session.status === "ended" && kind !== "summary") {
      notify("This session has ended. Create a new session to continue.");
      return;
    }
    const transcript = (
      kind === "summary" ? session.transcript : session.transcript.slice(-24)
    )
      .map(
        (t) =>
          `${session.mode === "Video" && t.startMs !== undefined && session.mediaStartedAt !== undefined ? `[${mediaTime(session, t.startMs)}] ` : ""}${t.source === "system" ? "System audio" : t.source === "microphone" ? "Microphone" : "Typed"}: ${t.text}`,
      )
      .join("\n");
    if (byteLength(transcript) > 120000) {
      notify(
        "This session is too long for a single recap. Export it to review the full transcript.",
      );
      return;
    }
    const id = uid();
    requestRef.current = { id, sessionId: session.id };
    let completeRequest!: () => void;
    const completion = new Promise<void>((resolve) => { completeRequest = resolve; });
    requestCompletionRef.current = { id, promise: completion };
    setGenerating(true);
    setStreamText("");
    setContextInsight("");
    setAnswerTab(kind);
    setAnswerIndex(null);
    if (kind === "answer") {
      if (session.status === "ready")
        updateSession(session.id, (s) => ({ ...s, status: "active" }));
      const duplicate = session.transcript.at(-1)?.text === q.trim();
      if (!duplicate)
        updateSession(session.id, (s) => ({
          ...s,
          transcript: [
            ...s.transcript,
            { id: uid(), text: q.trim(), source: "typed", time: Date.now() },
          ],
        }));
      setQuestion("");
    }
    try {
      const request: ChatRequest = {
        requestId: id,
        model:
          provider === "chatgpt"
            ? stateRef.current.settings.chatgptModel || ""
            : provider === "claude"
              ? "default"
              : stateRef.current.settings.model,
        provider,
        sessionMode: kind === "summary" && session.assistanceMode === "notes" && session.mode !== "Video" ? "Meeting" : session.mode,
        sessionTitle: session.title,
        role: session.role,
        company: session.company,
        question: q.trim(),
        context: session.context,
        instructions: session.instructions,
        transcript,
        kind,
      };
      if (
        kind === "answer" &&
        stateRef.current.settings.jevEnabled &&
        jevConfigured &&
        api.assessContext
      ) {
        try {
          const assessment = await api.assessContext(request);
          if (requestRef.current?.id !== id) return;
          if (
            assessment.confidence >= 0.7 &&
            assessment.decision === "clarify"
          ) {
            setContextInsight(
              "More context may help. The answer will flag missing details and suggest a clarification.",
            );
            const guidance =
              "\nAsk a brief clarification where the supplied facts are insufficient. Do not fill gaps with invented facts.";
            if (byteLength(request.instructions + guidance) <= 20000)
              request.instructions += guidance;
          } else if (
            assessment.confidence >= 0.7 &&
            assessment.decision === "notes" &&
            session.mode === "Meeting"
          ) {
            setContextInsight(
              "This discussion may be useful for meeting notes. You can capture it from the Notes tab.",
            );
          } else if (
            assessment.confidence >= 0.8 &&
            assessment.backgroundSupported < 0.4
          ) {
            setContextInsight(
              "The background may not support a personal example. Review the suggestion and add your own details.",
            );
          }
        } catch {
          if (requestRef.current?.id !== id) return;
          setContextInsight(
            "The optional context helper is unavailable. Your chosen AI will answer using the supplied background.",
          );
        }
      }
      if (requestRef.current?.id !== id) return;
      const result = await api.chat(request);
      if (requestRef.current?.id !== id) return;
      if (kind === "summary") {
        notesWatermarkRef.current[session.id] = session.transcript.length;
        updateSession(session.id, (s) => ({ ...s, summary: result.text }));
      } else
        updateSession(session.id, (s) => ({
          ...s,
          answers: [
            ...s.answers,
            {
              id,
              question: q.trim(),
              text: result.text,
              time: Date.now(),
              model: result.model,
              durationMs: result.durationMs,
            },
          ],
        }));
    } catch (e) {
      if (requestRef.current?.id === id) notify(errorText(e));
    } finally {
      if (requestRef.current?.id === id) {
        requestRef.current = null;
        setGenerating(false);
        setStreamText("");
        const pending = pendingAutoQuestion.current;
        if (
          pending &&
          listeningRef.current &&
          selectedRef.current === pending.sessionId &&
          autoAnswersEnabled(pending.sessionId)
        ) {
          pendingAutoQuestion.current = null;
          clearTimeout(autoTimer.current);
          autoTimer.current = setTimeout(
            () => void generateRef.current(pending.text),
            100,
          );
        }
      }
      if (requestCompletionRef.current?.id === id) requestCompletionRef.current = null;
      completeRequest();
    }
  }
  const generateRef = useRef(generate);
  generateRef.current = generate;
  useEffect(() => {
    if (!loaded) return;
    const timer = setInterval(() => {
      const session = stateRef.current.sessions.find((s) => s.id === selectedRef.current);
      if (
        session?.assistanceMode === "notes" &&
        session.status === "active" &&
        listeningRef.current &&
        !requestRef.current &&
        session.transcript.length > (notesWatermarkRef.current[session.id] || 0)
      ) void generateRef.current(notesPrompt(session), "summary");
    }, 60000);
    return () => clearInterval(timer);
  }, [loaded]);
  useEffect(() => {
    const unToken = api.onToken((event) => {
      if (requestRef.current?.id === event.requestId)
        setStreamText((text) => text + event.token);
    });
    const unTranscript = api.onTranscript((event) => {
      if (
        acceptingTranscriptRef.current !== event.sessionId ||
        selectedRef.current !== event.sessionId ||
        !event.text.trim()
      )
        return;
      const session = stateRef.current.sessions.find(
        (s) => s.id === event.sessionId,
      );
      if (!session || session.status !== "active") return;
      updateSession(event.sessionId, (s) => ({
        ...s,
        transcript: [...s.transcript, { id: uid(), ...event }],
      }));
      if (
        autoAnswersEnabled(event.sessionId) &&
        /\?|\b(tell me|walk me|explain|describe|how would|how do|what is|what are|why do|can you)\b/i.test(
          event.text,
        )
      ) {
        pendingAutoQuestion.current = {
          sessionId: event.sessionId,
          text: event.text,
        };
        clearTimeout(autoTimer.current);
        autoTimer.current = setTimeout(() => {
          const pending = pendingAutoQuestion.current;
          if (
            pending &&
            !requestRef.current &&
            selectedRef.current === pending.sessionId &&
            listeningRef.current &&
            autoAnswersEnabled(pending.sessionId)
          ) {
            pendingAutoQuestion.current = null;
            void generateRef.current(pending.text);
          }
        }, 1300);
      }
    });
    const unError = api.onVoiceError((event) => {
      if (event.sessionId === selectedRef.current) notify(event.error);
    });
    const unStatus = api.onVoiceStatus((event) => {
      if (event.sessionId === selectedRef.current) {
        listeningRef.current = event.listening;
        setListening(event.listening);
      }
    });
    const unProgress = api.onVoiceProgress(setVoiceProgress);
    const unQuit = api.onBeforeQuit(() => {
      if (quitting.current) return;
      quitting.current = true;
      void api
        .quitApp(loadedRef.current ? stateRef.current : null)
        .catch((e) => {
          quitting.current = false;
          notify(`Could not save before closing: ${errorText(e)}`);
        });
    });
    return () => {
      unToken();
      unTranscript();
      unError();
      unStatus();
      unProgress();
      unQuit();
      clearTimeout(autoTimer.current);
    };
  }, []);
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (
        (e.metaKey || e.ctrlKey) &&
        e.key === "Enter" &&
        view === "live" &&
        companion.mode === "main" &&
        companion.visible !== false &&
        answerTab === "answer" &&
        selected?.mode !== "Video" && selected?.assistanceMode !== "subtitles" &&
        modal === null
      ) {
        e.preventDefault();
        void generateRef.current(
          question || selected?.transcript.at(-1)?.text || "",
        );
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [
    question,
    selected,
    view,
    companion.mode,
    companion.visible,
    answerTab,
    modal,
  ]);

  async function cancel() {
    const request = requestRef.current;
    requestRef.current = null;
    setGenerating(false);
    setStreamText("");
    if (request) await api.cancelChat(request.id).catch(() => {});
  }
  async function stopAudio() {
    audioOperationRef.current = true;
    setAudioBusy(true);
    listeningRef.current = false;
    setListening(false);
    pendingAutoQuestion.current = null;
    clearTimeout(autoTimer.current);
    try {
      await api.stopListening();
    } catch (e) {
      notify(errorText(e));
    } finally {
      acceptingTranscriptRef.current = null;
      audioOperationRef.current = false;
      setAudioBusy(false);
    }
  }
  async function navigate(next: typeof view, id?: string) {
    if (id !== selectedRef.current || next !== "live") {
      await stopAudio();
      await cancel();
    }
    setSelectedId(id || null);
    setView(next);
    setQuestion("");
    setAnswerIndex(null);
    const nextSession = stateRef.current.sessions.find((session) => session.id === id);
    setAnswerTab(nextSession?.mode === "Video" ? "summary" : "answer");
  }
  async function openSession(session: InterviewSession) {
    if (session.status === "ready")
      updateSession(session.id, (s) => ({ ...s, status: "active" }));
    await navigate("live", session.id);
  }
  async function toggleAudio() {
    if (audioOperationRef.current) return;
    if (listeningRef.current) {
      const sessionId = selectedRef.current;
      await stopAudio();
      if (sessionId) await finalizeVideoNotes(sessionId);
      return;
    }
    if (!selected) return;
    await startAudio(selected.id);
  }
  async function startAudio(sessionId: string): Promise<boolean> {
    if (audioOperationRef.current) return false;
    if (listeningRef.current) return acceptingTranscriptRef.current === sessionId;
    if (!runtime.transcription) {
      notify("Set up local transcription in Settings first.");
      setBackgroundError("Set up local transcription in Settings before starting background listening.");
      await showWorkspace(true);
      return false;
    }
    const session = stateRef.current.sessions.find((item) => item.id === sessionId);
    const video = session?.mode === "Video";
    const macVideo = video && runtime.platform === "macos";
    const device = macVideo ? "system" : video ? videoInputs[sessionId] || "" : stateRef.current.settings.inputDevice;
    if (video && !macVideo && (!device || device === "system")) {
      notify("Route browser audio to a loopback or virtual input, then choose it in Audio input device.");
      return false;
    }
    audioOperationRef.current = true;
    setAudioBusy(true);
    const startedAt = Date.now();
    try {
      acceptingTranscriptRef.current = sessionId;
      await api.startListening({
        device,
        language: stateRef.current.settings.language,
        sessionId,
        source: macVideo ? "system" : video ? "microphone" : stateRef.current.settings.audioSource,
      });
      listeningRef.current = true;
      setListening(true);
      updateSession(sessionId, (s) => ({
        ...s, status: s.status === "ready" ? "active" : s.status,
        ...(s.mode === "Video" && s.mediaStartedAt === undefined ? { mediaStartedAt: startedAt } : {}),
      }));
      return true;
    } catch (e) {
      acceptingTranscriptRef.current = null;
      listeningRef.current = false;
      setListening(false);
      setBackgroundError(errorText(e));
      notify(errorText(e));
      return false;
    } finally {
      audioOperationRef.current = false;
      setAudioBusy(false);
    }
  }

  async function showWorkspace(settings = false) {
    companionVisibilityRevisionRef.current += 1;
    if (api.setCompanionMode) setCompanion(await api.setCompanionMode("main"));
    if (api.setCompanionVisible) setCompanion(await api.setCompanionVisible(true));
    setModal(null);
    setView(settings ? "settings" : selectedRef.current ? "live" : "workspace");
  }
  async function startBackgroundAssistance(mode: AssistanceMode) {
    if (!loadedRef.current) return;
    let session = stateRef.current.sessions.find((s) => s.id === selectedRef.current && s.status !== "ended");
    if (mode === "notes" && !listeningRef.current && session?.mode !== "Meeting" && session?.mode !== "Video")
      session = undefined;
    if (!session) {
      session = {
        id: uid(), title: "Meeting " + new Date().toLocaleDateString(),
        company: "", role: "", mode: "Meeting", createdAt: Date.now(),
        status: "ready", duration: 0, context: "", instructions: "",
        transcript: [], answers: [], assistanceMode: mode,
      };
      const next = { ...stateRef.current, sessions: [session, ...stateRef.current.sessions] };
      stateRef.current = next;
      setState(next);
      selectedRef.current = session.id;
      setSelectedId(session.id);
    } else {
      // Tray answer/listen actions must not enable automatic replies to video speech.
      if (session.mode === "Video" && mode !== "notes") mode = "subtitles";
      const next = {
        ...stateRef.current,
        sessions: stateRef.current.sessions.map((s) => s.id === session!.id ? { ...s, assistanceMode: mode } : s),
      };
      stateRef.current = next;
      setState(next);
    }
    pendingAutoQuestion.current = null;
    clearTimeout(autoTimer.current);
    pendingMeetingHintRef.current = null;
    setMeetingHint(null);
    setModal(null);
    setView("live");
    if (mode === "notes") setAnswerTab("summary");
    else setAnswerTab("answer");
    await startAudio(session.id);
    if (mode === "notes" && session.transcript.length && !requestRef.current)
      void generateRef.current(notesPrompt(session), "summary");
  }
  async function applyMeetingTrigger(hint?: MeetingHint) {
    const action = stateRef.current.settings.meetingAction || "none";
    if (action === "none") return;
    if (action === "prompt") {
      if (pendingMeetingHintRef.current) return;
      const pending = hint || { app: "Meeting trigger", detail: "Choose how NextCue can help." };
      pendingMeetingHintRef.current = pending;
      setMeetingHint(pending);
      await showWorkspace();
    } else await startBackgroundAssistance(action === "listen-notes" ? "notes" : "answers");
  }
  backgroundHandlerRef.current = async (event) => {
    if (!loadedRef.current) return;
    if (event.action === "show") await showWorkspace();
    else if (event.action === "settings") await showWorkspace(true);
    else if (event.action === "pause") {
      await stopAudio();
      // The final transcription event and React update may arrive in the same
      // turn as stopListening's reply. Let that update settle before the recap.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      const session = stateRef.current.sessions.find((s) => s.id === selectedRef.current);
      if (session?.mode === "Video") await finalizeVideoNotes(session.id);
      else if (session?.assistanceMode === "notes" && !requestRef.current &&
        session.transcript.length > (notesWatermarkRef.current[session.id] || 0))
        void generateRef.current(notesPrompt(session), "summary");
    }
    else if (event.action === "meeting-trigger") await applyMeetingTrigger();
    else await startBackgroundAssistance(event.action === "notes" ? "notes" : event.action === "answers" ? "answers" : "manual");
  };
  meetingHintHandlerRef.current = async (hint) => {
    if (!loadedRef.current || !stateRef.current.settings.meetingDetection || listeningRef.current) return;
    await applyMeetingTrigger(hint);
  };
  useEffect(() => {
    const enqueue = (operation: () => Promise<void>) => {
      backgroundQueueRef.current = backgroundQueueRef.current.then(operation).catch((e) => notify(errorText(e)));
    };
    const offAction = api.onBackgroundAction?.((event) => enqueue(() => backgroundHandlerRef.current(event)));
    const offHint = api.onMeetingHint?.((event) => enqueue(() => meetingHintHandlerRef.current(event)));
    return () => { offAction?.(); offHint?.(); };
  }, []);
  useEffect(() => {
    if (!loaded || !initialChecksComplete) return;
    const config: BackgroundConfiguration = {
      runInBackground: stateRef.current.settings.runInBackground !== false,
      startHidden: stateRef.current.settings.startHidden === true,
      meetingDetection: stateRef.current.settings.meetingDetection === true,
      meetingAction: stateRef.current.settings.meetingAction || "none",
    };
    void (async () => {
      try {
        if (api.configureBackground) setBackground(await api.configureBackground(config));
        if (api.getCompanionStatus) setCompanion(await api.getCompanionStatus());
        await api.backgroundReady?.();
      } catch (e) { setBackgroundError(errorText(e)); }
    })();
  }, [loaded, initialChecksComplete]);
  useEffect(() => {
    if (loaded) void api.updateBackgroundActivity?.({ listening, mode: selected?.assistanceMode || "manual" }).catch(() => {});
  }, [loaded, listening, selected?.assistanceMode]);
  useEffect(() => {
    if (view !== "settings" || companion.visible === false) setBrowserConnection(null);
  }, [view, companion.visible]);
  async function saveBackgroundSettings(patch: Partial<BackgroundConfiguration>) {
    if (!api.configureBackground || backgroundSavingRef.current) return;
    backgroundSavingRef.current = true;
    setBackgroundSaving(true);
    setBackgroundError("");
    const config: BackgroundConfiguration = {
      runInBackground: state.settings.runInBackground !== false,
      startHidden: state.settings.startHidden === true,
      meetingDetection: state.settings.meetingDetection === true,
      meetingAction: state.settings.meetingAction || "none",
      ...patch,
    };
    try {
      const accepted = await api.configureBackground(config);
      setBackground(accepted);
      updateSettings({
        runInBackground: accepted.runInBackground, startHidden: accepted.startHidden,
        meetingDetection: accepted.meetingDetection, meetingAction: accepted.meetingAction,
      });
    } catch (e) { setBackgroundError(errorText(e)); }
    finally { backgroundSavingRef.current = false; setBackgroundSaving(false); }
  }
  async function endSession() {
    if (!selected) return;
    await stopAudio();
    if (selected.mode !== "Video") await cancel();
    updateSession(selected.id, (s) => ({ ...s, status: "ended" }));
    await finalizeVideoNotes(selected.id);
  }
  async function setupVoice() {
    setVoiceSetup(true);
    setVoiceProgress({ progress: 0, status: "Preparing local speech model…" });
    try {
      await api.setupVoice();
      await refresh();
      notify("Local transcription is ready.");
    } catch (e) {
      notify(errorText(e));
    } finally {
      setVoiceSetup(false);
    }
  }
  async function importContext() {
    try {
      const file = await api.importContext();
      if (file) setDraft((d) => ({ ...d, context: file.text }));
    } catch (e) {
      notify(errorText(e));
    }
  }
  function createSession(e: FormEvent) {
    e.preventDefault();
    const { mediaOffsetSeconds: offsetDraft, ...fields } = draft;
    const mediaOffsetSeconds = Number(offsetDraft);
    if (draft.mode === "Video" && (!Number.isFinite(mediaOffsetSeconds) || mediaOffsetSeconds < 0 || mediaOffsetSeconds > 359999)) {
      notify("Enter a video starting position between 0 and 359999 seconds.");
      return;
    }
    const session: InterviewSession = {
      id: uid(),
      ...fields,
      title:
        draft.title.trim() ||
        `${draft.company.trim() || "New"} ${draft.mode.toLowerCase()}`,
      role: draft.role.trim(),
      company: draft.company.trim(),
      createdAt: Date.now(),
      status: "ready",
      duration: 0,
      transcript: [],
      answers: [],
      ...(draft.mode === "Video" ? { assistanceMode: "notes", mediaOffsetSeconds } : {}),
    };
    const inputError = sessionInputError(session);
    if (inputError) {
      notify(inputError);
      return;
    }
    setState((s) => ({ ...s, sessions: [session, ...s.sessions] }));
    setModal(null);
    void navigate("live", session.id);
  }
  function newSession(mode: SessionMode = "Interview") {
    setDraft({
      title: "",
      company: "",
      role: "",
      mode,
      context: "",
      instructions: "",
      mediaOffsetSeconds: "0",
    });
    setModal("create");
  }
  const currentAnswer =
    selected?.answers[answerIndex ?? selected.answers.length - 1];
  const visibleSessions = state.sessions.filter(
    (s) =>
      (filter === "all" || s.status === filter) &&
      `${s.title} ${s.company} ${s.role}`
        .toLowerCase()
        .includes(search.toLowerCase()),
  );

  const answerProvider = state.settings.answerProvider || "local";
  const providerLabel =
    answerProvider === "chatgpt"
      ? "ChatGPT"
      : answerProvider === "claude"
        ? "Claude"
        : "Local AI";
  const localReady =
    runtime.ollama &&
    runtime.models.some((m) => m.name === state.settings.model);
  const aiReady =
    answerProvider === "chatgpt"
      ? chatgpt.connected &&
        chatgpt.planUsage &&
        chatgpt.models.some((m) => m.slug === state.settings.chatgptModel)
      : answerProvider === "claude"
        ? claude.connected && claude.subscription
        : localReady;
  const latestQuestion = selected?.transcript.at(-1)?.text || "";
  const videoSubtitles = selected?.mode === "Video" && selected.assistanceMode === "subtitles";
  const videoTranscript = selected?.transcript.filter((entry) => entry.source !== "typed") || [];
  const latestCaption = videoTranscript.at(-1);
  const canExportSubtitles = !!selected?.mediaStartedAt && videoTranscript.some((entry) =>
    entry.startMs !== undefined && entry.endMs !== undefined && entry.endMs > entry.startMs,
  );
  const displayShortcut = (value: string) =>
    value
      .replace(
        /CommandOrControl/gi,
        runtime.platform === "macos" ? "⌘" : "Ctrl",
      )
      .replace(/Command|Super/gi, "⌘")
      .replace(/Control/gi, "Ctrl")
      .replace(/\+/g, " + ");
  const shortcut = displayShortcut(
    companion.answerShortcut ||
      state.settings.answerShortcut ||
      defaults.settings.answerShortcut!,
  );
  const visibilityShortcut = displayShortcut(
    companion.visibilityShortcut ||
      state.settings.visibilityShortcut ||
      defaults.settings.visibilityShortcut!,
  );
  const emergencyShortcut =
    runtime.platform === "macos" ? "⌘ + Shift + H" : "Ctrl + Shift + H";
  const captureLabel =
    companion.captureProtection === "disabled"
      ? "Visible in screen captures"
      : companion.captureProtection === "best-effort"
        ? "Capture protection: best effort"
        : "May appear in screen sharing";
  const answerText = generating
    ? streamText
    : answerTab === "summary"
      ? selected?.summary
      : currentAnswer?.text;
  const latestAnswer = selected?.answers.at(-1);
  const lastSession = state.sessions.find((s) => s.status !== "ended");
  const recentSessions = state.sessions.slice(0, 4);

  async function setCompanionMode(mode: CompanionMode) {
    if (companionBusyRef.current) return false;
    if (mode !== "main" && !selectedRef.current) {
      notify("Open a session first to use Pin mode.");
      return false;
    }
    if (!api.setCompanionMode) {
      notify("Pin mode is available in the updated desktop app.");
      return false;
    }
    companionVisibilityRevisionRef.current += 1;
    companionBusyRef.current = true;
    setCompanionBusy(true);
    try {
      const result = await api.setCompanionMode(mode);
      setCompanion(result);
      setModal(null);
      return true;
    } catch (e) {
      // A native operation can reject after restoring the workspace. Keep the
      // rendered mode aligned with that rollback instead of leaving a tiny UI.
      try {
        const restored = await api.setCompanionMode("main");
        setCompanion(restored);
      } catch {
        // Keep the existing controls available if the recovery also fails.
      }
      notify(errorText(e));
      return false;
    } finally {
      companionBusyRef.current = false;
      setCompanionBusy(false);
    }
  }
  async function showCompanionAnswer() {
    setAnswerIndex(null);
    if (!(await setCompanionMode("answer"))) return;
    const session = stateRef.current.sessions.find(
      (s) => s.id === selectedRef.current,
    );
    if (session?.mode === "Video" || session?.assistanceMode === "subtitles") {
      if (session.mode === "Video") setAnswerTab("summary");
      return;
    }
    const latest = session?.transcript.at(-1);
    const previousAnswer = session?.answers.at(-1);
    if (
      session?.status !== "ended" &&
      latest?.text &&
      !requestRef.current &&
      (latest.text !== previousAnswer?.question ||
        latest.time > (previousAnswer?.time || 0))
    ) {
      void generateRef.current(latest.text);
    }
  }
  const companionToggleRef = useRef(() => {});
  companionToggleRef.current = () => {
    if (companion.mode === "answer") void setCompanionMode("pin");
    else void showCompanionAnswer();
  };
  useEffect(
    () => api.onCompanionToggle?.(() => companionToggleRef.current()),
    [],
  );
  useEffect(() => {
    document.body.dataset.companion = companion.mode;
    return () => {
      delete document.body.dataset.companion;
    };
  }, [companion.mode]);
  useEffect(() => {
    if (
      loaded &&
      runtime.models.length &&
      !runtime.models.some((m) => m.name === stateRef.current.settings.model)
    )
      updateSettings({ model: runtime.models[0].name });
  }, [loaded, runtime.models]);
  useEffect(() => {
    if (!loaded) return;
    const configuration: CompanionConfiguration = {
      captureProtection: stateRef.current.settings.captureProtection !== false,
      clickThrough: stateRef.current.settings.clickThrough === true,
      answerShortcut:
        stateRef.current.settings.answerShortcut ||
        defaults.settings.answerShortcut!,
      visibilityShortcut:
        stateRef.current.settings.visibilityShortcut ||
        defaults.settings.visibilityShortcut!,
      meetingShortcut:
        stateRef.current.settings.meetingShortcut || defaults.settings.meetingShortcut!,
    };
    setShortcutDraft({
      answerShortcut: configuration.answerShortcut,
      visibilityShortcut: configuration.visibilityShortcut,
      meetingShortcut: configuration.meetingShortcut!,
    });
    if (api.configureCompanion)
      void api
        .configureCompanion(configuration)
        .then(setCompanion)
        .catch(async (e) => {
          setShortcutError(errorText(e));
          if (api.getCompanionStatus) {
            try {
              const accepted = await api.getCompanionStatus();
              setCompanion(accepted);
              updateSettings({
                captureProtection: accepted.captureProtectionEnabled ?? true,
                clickThrough: accepted.clickThrough ?? stateRef.current.settings.clickThrough ?? false,
                answerShortcut:
                  accepted.answerShortcut || defaults.settings.answerShortcut!,
                visibilityShortcut:
                  accepted.visibilityShortcut ||
                  defaults.settings.visibilityShortcut!,
                meetingShortcut: accepted.meetingShortcut || defaults.settings.meetingShortcut!,
              });
            } catch {
              // The failure stays visible in Settings; hiding is guarded natively.
            }
          }
        });
  }, [loaded]);
  useEffect(() => {
    const unsubscribe = api.onCompanionVisibility?.((visible) => {
      const revision = ++companionVisibilityRevisionRef.current;
      setCompanion((previous) => ({ ...previous, visible }));
      if (!visible || !api.getCompanionStatus) return;
      // Recovery may restore a click-through Pin/Answer window to Main so
      // pointer controls work again. Read that native mode before rendering it.
      void api.getCompanionStatus().then((status) => {
        if (companionVisibilityRevisionRef.current !== revision) return;
        setCompanion((previous) => ({ ...previous, ...status, visible: true }));
      }).catch((error) => {
        if (companionVisibilityRevisionRef.current === revision) notify(errorText(error));
      });
    });
    return () => {
      companionVisibilityRevisionRef.current += 1;
      unsubscribe?.();
    };
  }, []);
  async function saveCompanionSettings(patch: Partial<CompanionConfiguration>) {
    if (!api.configureCompanion || shortcutSaving) return;
    companionVisibilityRevisionRef.current += 1;
    setShortcutSaving(true);
    setShortcutError("");
    const configuration: CompanionConfiguration = {
      captureProtection: state.settings.captureProtection !== false,
      clickThrough: state.settings.clickThrough === true,
      answerShortcut:
        state.settings.answerShortcut || defaults.settings.answerShortcut!,
      visibilityShortcut:
        state.settings.visibilityShortcut ||
        defaults.settings.visibilityShortcut!,
      meetingShortcut: state.settings.meetingShortcut || defaults.settings.meetingShortcut!,
      ...patch,
    };
    try {
      const result = await api.configureCompanion(configuration);
      setCompanion(result);
      const saved = {
        captureProtection: configuration.captureProtection,
        clickThrough: result.clickThrough ?? configuration.clickThrough ?? false,
        answerShortcut:
          result.answerShortcut ||
          configuration.answerShortcut ||
          defaults.settings.answerShortcut!,
        visibilityShortcut:
          result.visibilityShortcut ||
          configuration.visibilityShortcut ||
          defaults.settings.visibilityShortcut!,
        meetingShortcut: result.meetingShortcut || configuration.meetingShortcut || defaults.settings.meetingShortcut!,
      };
      updateSettings(saved);
      if (
        patch.answerShortcut !== undefined ||
        patch.visibilityShortcut !== undefined ||
        patch.meetingShortcut !== undefined
      )
        setShortcutDraft({
          answerShortcut: saved.answerShortcut,
          visibilityShortcut: saved.visibilityShortcut,
          meetingShortcut: saved.meetingShortcut,
        });
      notify("Assistant settings saved.");
    } catch (e) {
      setShortcutError(errorText(e));
    } finally {
      setShortcutSaving(false);
    }
  }
  async function hideAssistant() {
    if (!api.setCompanionVisible) {
      notify("Hide / show is available in the updated desktop app.");
      return;
    }
    companionVisibilityRevisionRef.current += 1;
    try {
      const result = await api.setCompanionVisible(false);
      setCompanion(result);
    } catch (e) {
      notify(errorText(e));
    }
  }
  async function takeNotesInBackground() {
    if (backgroundNotesBusyRef.current || !selected || selected.status === "ended") return;
    if (!api.setCompanionVisible) {
      notify("Background notes are available in the updated desktop app.");
      return;
    }
    if (!aiReady) {
      notify("Connect your selected AI in Settings before taking notes in background.");
      await showWorkspace(true);
      return;
    }
    backgroundNotesBusyRef.current = true;
    setBackgroundNotesBusy(true);
    try {
      pendingAutoQuestion.current = null;
      clearTimeout(autoTimer.current);
      await cancel();
      const next = {
        ...stateRef.current,
        sessions: stateRef.current.sessions.map((session) => session.id === selected.id ? { ...session, assistanceMode: "notes" as const } : session),
      };
      stateRef.current = next;
      setState(next);
      setAnswerTab("summary");
      const started = listeningRef.current || await startAudio(selected.id);
      if (!started || !listeningRef.current) return;
      await hideAssistant();
    } finally {
      backgroundNotesBusyRef.current = false;
      setBackgroundNotesBusy(false);
    }
  }
  const editContext = () => {
    if (!selected) return;
    setDraft((d) => ({
      ...d,
      context: selected.context,
      instructions: selected.instructions,
    }));
    setModal("context");
  };
  const copyAnswer = () => {
    if (answerText)
      void (
        api.copyText
          ? api.copyText(answerText)
          : navigator.clipboard.writeText(answerText)
      )
        .then(() => notify("Answer copied."))
        .catch((e) => notify(errorText(e)));
  };
  async function setVideoAssistance(mode: "notes" | "subtitles") {
    if (!selected) return;
    pendingAutoQuestion.current = null;
    clearTimeout(autoTimer.current);
    updateSession(selected.id, (session) => ({ ...session, assistanceMode: mode }));
    setAnswerTab("summary");
    if (mode === "subtitles") await cancel();
  }
  async function finalizeVideoNotes(sessionId: string) {
    // A stopped recorder drains its final segment before replying. Allow the
    // transcript render to settle, then wait out any notes using an older snapshot.
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    while (requestRef.current && selectedRef.current === sessionId) {
      const completion = requestCompletionRef.current;
      if (!completion || completion.id !== requestRef.current.id) return;
      await completion.promise;
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    const session = stateRef.current.sessions.find((item) => item.id === sessionId);
    if (selectedRef.current !== sessionId || session?.mode !== "Video" || session.assistanceMode !== "notes") return;
    if (session.transcript.length > (notesWatermarkRef.current[sessionId] || 0)) {
      await generateRef.current(notesPrompt(session), "summary");
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
  }
  async function exportMarkdown() {
    if (!selected) return;
    const sessionId = selected.id;
    if (listeningRef.current) await stopAudio();
    if (selected.mode === "Video") await finalizeVideoNotes(sessionId);
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const session = stateRef.current.sessions.find((item) => item.id === sessionId);
    if (!session) return;
    try {
      if (await api.exportSession(session)) notify("Session exported.");
    } catch (error) { notify(errorText(error)); }
  }
  async function exportSubtitles(format: "srt" | "vtt") {
    if (!selected || !api.exportSubtitles) return;
    if (listeningRef.current) {
      await stopAudio();
      // Native stop flushes the final speech chunk. Let its React update settle.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    await finalizeVideoNotes(selected.id);
    const session = stateRef.current.sessions.find((item) => item.id === selected.id);
    if (!session) return;
    try {
      if (await api.exportSubtitles(session, format)) notify(`${format.toUpperCase()} subtitles exported.`);
    } catch (error) { notify(errorText(error)); }
  }
  const sessionRow = (s: InterviewSession) => (
    <div className="session-row" key={s.id}>
      <div className={"session-icon mode-" + s.mode.toLowerCase()}>
        {s.mode === "Video" ? (
          <Video size={19} />
        ) : s.mode === "Meeting" ? (
          <MessageCircle size={19} />
        ) : s.mode === "Practice" ? (
          <Mic size={19} />
        ) : (
          <AudioLines size={19} />
        )}
      </div>
      <button className="session-info" onClick={() => void openSession(s)}>
        <strong>{s.title}</strong>
        <small>
          {s.role || s.mode}
          {s.company ? " · " + s.company : ""} · {date(s.createdAt)}
        </small>
      </button>
      <span className={"session-status " + s.status}>
        {s.status === "ended"
          ? "Completed"
          : s.status === "active"
            ? "In progress"
            : "Ready"}
      </span>
      <div className="session-actions">
        <button
          className="button secondary small-button"
          onClick={() => void openSession(s)}
        >
          {s.status === "ended"
            ? "Review"
            : view === "history"
              ? "Open"
              : "Continue"}
          <ArrowRight size={14} />
        </button>
      </div>
    </div>
  );
  const audioButton = (
    <button
      className={"button " + (listening ? "secondary" : "primary")}
      disabled={audioBusy || selected?.status === "ended"}
      onClick={() => void toggleAudio()}
    >
      {audioBusy ? (
        <Loader2 size={16} className="spin" />
      ) : listening ? (
        <Pause size={16} />
      ) : (
        <Mic size={16} />
      )}
      {listening ? "Pause listening" : "Start listening"}
    </button>
  );
  const answerBody = generating ? (
    <>
      <div className="answer-label">
        <Loader2 size={14} className="spin" />
        {streamText ? "Writing your suggestion" : "Thinking on your device…"}
      </div>
      <div className="markdown streaming">
        <Markdown>{streamText}</Markdown>
        {!streamText && (
          <div className="skeleton-text">
            <span />
            <span />
            <span />
          </div>
        )}
      </div>
    </>
  ) : answerTab === "summary" && selected?.summary ? (
    <>
      <div className="answer-label">
        {selected?.mode === "Video" ? "VIDEO NOTES" : selected?.mode === "Meeting" ? "MEETING NOTES" : "SESSION RECAP"}
      </div>
      <div className="markdown">
        <Markdown>{selected.summary}</Markdown>
      </div>
    </>
  ) : answerTab === "answer" && currentAnswer ? (
    <>
      <div className="answer-label">
        <Sparkles size={14} />
        SUGGESTED ANSWER <span>{time(currentAnswer.time)}</span>
      </div>
      <div className="answered-question">{currentAnswer.question}</div>
      <div className="markdown">
        <Markdown>{currentAnswer.text}</Markdown>
      </div>
    </>
  ) : (
    <div className="answer-empty">
      <img src="./logo.svg" alt="" className="welcome-mark" />
      <h2>
        {answerTab === "summary"
          ? "Keep the useful parts."
          : "Find your words."}
      </h2>
      <p>
        {answerTab === "summary"
          ? selected?.mode === "Video" ? "Capture the video audio, then turn it into key points and useful details." : "Turn your conversation into key points and next steps."
          : "Listen to a question or type one below. Your assistant will help you shape an answer."}
      </p>
      {answerTab === "answer" && selected?.status !== "ended" && (
        <div className="question-chips">
          {(selected?.mode === "Meeting"
            ? [
                "What should we clarify before deciding?",
                "What are the next steps?",
              ]
            : ["Tell me about yourself.", "How do you handle a challenge?"]
          ).map((q) => (
            <button key={q} onClick={() => setQuestion(q)}>
              {q}
              <ArrowRight size={13} />
            </button>
          ))}
        </div>
      )}
      {answerTab === "summary" && (
        <button
          className="button primary"
          disabled={!selected?.transcript.length || generating}
          onClick={() => void generate(selected?.mode === "Video" ? notesPrompt(selected) : "Summarize this session.", "summary")}
        >
          <Sparkles size={16} />
          {selected?.mode === "Video" ? "Create video notes" : selected?.mode === "Meeting"
            ? "Create meeting notes"
            : "Generate recap"}
        </button>
      )}
    </div>
  );
  const answerRegionLabel =
    videoSubtitles ? "Latest caption" : answerTab === "summary"
      ? selected?.mode === "Video" ? "Video notes" : selected?.mode === "Meeting"
        ? "Meeting notes"
        : "Session recap"
      : "Suggested answer";

  // Keep session hooks mounted, but remove private content and controls from
  // the webview whenever the native window is hidden. Recorders can still
  // retain earlier frames; this is not a capture exclusion guarantee.
  if (companion.visible === false)
    return <div className="assistant-hidden" aria-hidden="true" />;

  if (companion.mode !== "main" && selected)
    return (
      <div
        className={
          "companion-shell " +
          (companion.mode === "pin" ? "pin-shell" : "answer-shell")
        }
      >
        {companion.mode === "pin" ? (
          <div className="pin-controls">
            <div
              className="companion-drag"
              title="Drag to move pin"
              onMouseDown={(event) => {
                if (event.button === 0)
                  void api.startDragging?.().catch((e) => notify(errorText(e)));
              }}
            >
              <GripHorizontal size={12} />
            </div>
            <button
              className={
                "companion-pin " +
                (generating ? "thinking" : listening ? "is-listening" : "")
              }
              aria-label={selected.mode === "Video" ? "Show video assistant" : "Show answer"}
              title={(selected.mode === "Video" ? "Show video assistant · " : "Show answer · ") + (listening ? "Listening" : "Paused")}
              disabled={companionBusy}
              onClick={() => void showCompanionAnswer()}
            >
              <img src="./logo.svg" alt="" />
              <span
                className={
                  "pin-state " +
                  (generating ? "thinking" : listening ? "listening" : "paused")
                }
              />
            </button>
            {toast && (
              <span className="pin-tip" role="status" title={toast}>
                !
              </span>
            )}
          </div>
        ) : (
          <section
            className="companion-card"
            aria-label="NextCue floating assistant"
          >
            <header className="companion-header">
              <div
                className="companion-drag"
                onMouseDown={(event) => {
                  if (event.button === 0)
                    void api
                      .startDragging?.()
                      .catch((e) => notify(errorText(e)));
                }}
              >
                <img src="./logo.svg" alt="" />
                <strong>NextCue</strong>
                <GripHorizontal size={16} />
              </div>
              <button
                className="icon-button"
                aria-label="Open full app"
                title="Open full app"
                disabled={companionBusy}
                onClick={() => void setCompanionMode("main")}
              >
                <Maximize2 size={15} />
              </button>
              <button
                className="icon-button"
                aria-label="Hide assistant"
                title={"Hide assistant · " + visibilityShortcut}
                disabled={companion.visibilityShortcutAvailable === false && !background.trayAvailable}
                onClick={() => void hideAssistant()}
              >
                <EyeOff size={15} />
              </button>
              <button
                className="icon-button"
                aria-label="Collapse answer"
                title="Collapse to pin"
                disabled={companionBusy}
                onClick={() => void setCompanionMode("pin")}
              >
                <X size={16} />
              </button>
            </header>
            <div className="companion-state">
              <span className={"dot " + (listening ? "green" : "gray")} />
              {listening
                ? "Listening"
                : selected.status === "ended"
                  ? "Session ended"
                  : "Listening paused"}
              <span>{duration(selected.duration)}</span>
            </div>
            <div
              className="companion-content"
              role="region"
              aria-label={answerRegionLabel}
            >
              {videoSubtitles ? (
                <div className="floating-caption" aria-live="polite">
                  <div className="answer-label"><Captions size={14} />LATEST CAPTION</div>
                  <p>{latestCaption?.text || "Start listening to capture spoken captions from the video."}</p>
                </div>
              ) : generating ||
              latestAnswer ||
              (answerTab === "summary" && selected.summary) ? (
                answerBody
              ) : (
                <div className="companion-empty">
                  <Sparkles size={26} />
                  <h3>{selected.mode === "Video" ? "Capture the useful parts." : "Ready when you are."}</h3>
                  <p>
                    {latestQuestion || (selected.mode === "Video"
                      ? "Start listening to capture video speech, then create notes."
                      : "Start listening in the full app, then click the pin when you need an answer.")}
                  </p>
                </div>
              )}
            </div>
            <div className="companion-actions">
              {videoSubtitles ? null : generating ? (
                <button
                  className="button secondary"
                  onClick={() => void cancel()}
                >
                  <Square size={13} />
                  Stop generating
                </button>
              ) : (
                <button
                  className="button primary"
                  disabled={!latestQuestion || (selected.mode !== "Video" && selected.status === "ended")}
                  onClick={() => void generate(selected.mode === "Video" ? notesPrompt(selected) : latestQuestion, selected.mode === "Video" ? "summary" : "answer")}
                >
                  <Sparkles size={14} />
                  {selected.mode === "Video" ? "Create video notes" : "Get answer"}
                </button>
              )}
              <button
                className="icon-button"
                aria-label={videoSubtitles ? "Copy caption" : "Copy answer"}
                title={videoSubtitles ? "Copy caption" : "Copy answer"}
                disabled={videoSubtitles ? !latestCaption : !answerText}
                onClick={videoSubtitles ? () => void (api.copyText ? api.copyText(latestCaption?.text || "") : navigator.clipboard.writeText(latestCaption?.text || "")).catch((error) => notify(errorText(error))) : copyAnswer}
              >
                <Copy size={16} />
              </button>
              <button
                className="icon-button"
                aria-label={listening ? "Pause listening" : "Start listening"}
                title={listening ? "Pause listening" : "Start listening"}
                disabled={audioBusy || selected.status === "ended"}
                onClick={() => void toggleAudio()}
              >
                {listening ? <Pause size={16} /> : <Mic size={16} />}
              </button>
            </div>
            {(selected.mode === "Meeting" || selected.mode === "Video") && selected.status !== "ended" && <button className="button secondary floating-background-notes" disabled={backgroundNotesBusy || audioBusy} onClick={() => void takeNotesInBackground()}><EyeOff size={14} />{backgroundNotesBusy ? "Starting background notes…" : "Take notes in background"}</button>}
            <footer className="companion-footer">
              <span title="Verify your recording or share preview. Capture exclusion varies by operating system and recording app.">
                <ShieldCheck size={11} />
                {captureLabel}
              </span>
              {companion.shortcutAvailable !== false && (
                <small>{shortcut}</small>
              )}
            </footer>
            {toast && (
              <div className="companion-toast" role="status">
                {toast}
              </div>
            )}
          </section>
        )}
      </div>
    );

  return (
    <div className="app-shell">
      <aside className="sidebar nav-rail" inert={modal !== null}>
        <div className="window-drag" data-tauri-drag-region />
        <button
          className="rail-brand"
          title="NextCue home"
          aria-label="NextCue home"
          onClick={() => void navigate("workspace")}
        >
          <img src="./logo.svg" alt="" />
        </button>
        <nav className="rail-nav" aria-label="Main navigation">
          <button
            className={
              "rail-button " +
              (view === "workspace" || view === "live" ? "active" : "")
            }
            onClick={() => void navigate("workspace")}
            title="Home"
          >
            <Home size={21} />
            <span>Home</span>
          </button>
          <button
            className={"rail-button " + (view === "history" ? "active" : "")}
            onClick={() => void navigate("history")}
            title="All sessions"
          >
            <History size={21} />
            <span>History</span>
          </button>
        </nav>
        <div className="rail-bottom">
          <button
            className="rail-button"
            title="Getting started"
            onClick={() => setModal("help")}
          >
            <BookOpen size={20} />
            <span>Help</span>
          </button>
          <button
            className={"rail-button " + (view === "settings" ? "active" : "")}
            title="Settings"
            onClick={() => void navigate("settings")}
          >
            <Settings2 size={20} />
            <span>Settings</span>
          </button>
        </div>
      </aside>
      <main className="main-content desktop-main" inert={modal !== null}>
        <header className="topbar app-topbar" data-tauri-drag-region>
          <div className="topbar-title">
            <strong>NextCue</strong>
            <span>
              {view === "live"
                ? "Your conversation"
                : view === "settings"
                  ? "Set up your assistant"
                  : view === "history"
                    ? "Session library"
                    : "Your desktop assistant"}
            </span>
          </div>
          <div className="topbar-actions">
            <span
              className={"ready-badge " + (aiReady ? "ready" : "needs-setup")}
            >
              <span className="dot" />
              {aiReady ? "AI ready" : "AI setup needed"}
            </span>
            <button
              className="icon-button"
              aria-label="Refresh connection"
              title="Refresh connection"
              disabled={checking}
              onClick={() => {
                void refresh();
                if (answerProvider !== "local") void checkProviderConnection();
              }}
            >
              <RefreshCw size={15} className={checking ? "spin" : ""} />
            </button>
          </div>
        </header>
        {meetingHint && (
          <section className="meeting-trigger-prompt" aria-label="Meeting detected">
            <Bell size={20} />
            <div>
              <strong>{meetingHint.app} is ready. How can NextCue help?</strong>
              <p>{meetingHint.detail || "A meeting hint is available. Choose an action when you are ready."}</p>
            </div>
            <div className="meeting-trigger-actions">
              {([
                ["manual", "Start listening"], ["notes", "Take notes"], ["answers", "Answer questions"],
              ] as const).map(([mode, label]) => (
                <button key={mode} className="button secondary small-button" disabled={audioBusy}
                  onClick={() => {
                    backgroundQueueRef.current = backgroundQueueRef.current
                      .then(() => startBackgroundAssistance(mode))
                      .catch((e) => notify(errorText(e)));
                  }}>{label}</button>
              ))}
              <button className="icon-button" aria-label="Dismiss meeting hint" onClick={() => {
                pendingMeetingHintRef.current = null;
                setMeetingHint(null);
              }}><X size={16} /></button>
            </div>
          </section>
        )}
        {!loaded ? (
          <div className="loading-state">
            <Loader2 className="spin" />
            Opening your assistant…
          </div>
        ) : view === "settings" ? (
          <div className="settings-page page">
            <div className="page-heading">
              <div>
                <span className="eyebrow">YOUR ASSISTANT, YOUR WAY</span>
                <h1>Make yourself at home.</h1>
                <p>Connect your AI, choose audio, and you're ready.</p>
              </div>
            </div>
            <div className="settings-grid settings-layout">
              <section className="panel settings-panel">
                <div className="panel-title">
                  <span className="setup-number">1</span>
                  <div>
                    <h2>Your AI</h2>
                    <p>Choose what powers answers in NextCue.</p>
                  </div>
                  <span
                    className={
                      "status-badge " + (aiReady ? "connected" : "neutral")
                    }
                  >
                    {aiReady ? "Ready" : "Setup needed"}
                  </span>
                </div>
                <label>
                  Answer provider
                  <select
                    aria-label="Answer provider"
                    value={answerProvider}
                    disabled={generating || signingIn || providerBusy}
                    onChange={(event) => {
                      updateSettings({
                        answerProvider: event.target.value as AnswerProvider,
                      });
                      setProviderError("");
                    }}
                  >
                    <option value="local">Local AI · Ollama</option>
                    <option value="chatgpt">ChatGPT · existing account</option>
                    <option value="claude">
                      Claude · existing Claude Code login
                    </option>
                  </select>
                </label>
                {answerProvider === "local" && (
                  <>
                    {!runtime.ollama && (
                      <div className="setup-intro">
                        <Cpu size={30} />
                        <h3>Connect your thinking partner.</h3>
                        <p>
                          Install Ollama and a local model first, then start it
                          here.
                        </p>
                        <button
                          className="button primary"
                          disabled={checking}
                          onClick={async () => {
                            setChecking(true);
                            try {
                              await api.startOllama();
                              await refresh();
                            } catch (e) {
                              notify(errorText(e));
                              setChecking(false);
                            }
                          }}
                        >
                          {checking ? (
                            <Loader2 size={16} className="spin" />
                          ) : (
                            <Play size={16} />
                          )}
                          Start Ollama
                        </button>
                        {runtime.error && (
                          <p className="connection-error">{runtime.error}</p>
                        )}
                      </div>
                    )}
                    <label>
                      AI model
                      <select
                        value={state.settings.model}
                        onChange={(e) =>
                          updateSettings({ model: e.target.value })
                        }
                      >
                        {!runtime.models.length && (
                          <option value={state.settings.model}>
                            No installed models found
                          </option>
                        )}
                        {runtime.models.map((m) => (
                          <option key={m.name} value={m.name}>
                            {m.name}
                          </option>
                        ))}
                      </select>
                    </label>
                    {runtime.ollama && !runtime.models.length && (
                      <div className="connection-error">
                        <p>Install a model in Ollama, then refresh.</p>
                        <code>ollama pull qwen2.5:7b-instruct</code>
                      </div>
                    )}
                    <p className="helper-line">
                      Smaller models usually answer faster.
                    </p>
                    <details className="advanced-settings">
                      <summary>Advanced connection settings</summary>
                      <label>
                        Ollama URL
                        <input
                          value={state.settings.ollamaUrl}
                          onChange={(e) =>
                            updateSettings({ ollamaUrl: e.target.value })
                          }
                        />
                      </label>
                      <p className="helper-line">
                        Only a local Ollama server is supported.
                      </p>
                    </details>
                  </>
                )}
                {answerProvider === "chatgpt" && (
                  <div className="provider-connection">
                    <div className="provider-account">
                      <Sparkles size={24} />
                      <div>
                        <h3>
                          {chatgpt.connected
                            ? "ChatGPT connected"
                            : "Bring your ChatGPT account"}
                        </h3>
                        <p>
                          {chatgpt.connected
                            ? chatgpt.email || "Connected account"
                            : "Sign in and approve ChatGPT plan usage. Answers return here and in your pin."}
                        </p>
                      </div>
                    </div>
                    {!!chatgpt.accounts?.length && (
                      <label>
                        Saved account
                        <select
                          aria-label="ChatGPT account"
                          value={chatgpt.activeAccountId || ""}
                          disabled={generating || signingIn || providerBusy}
                          onChange={async (event) => {
                            if (!api.selectChatgptAccount) return;
                            setProviderBusy(true);
                            setProviderError("");
                            try {
                              acceptChatgpt(
                                await api.selectChatgptAccount(
                                  event.target.value,
                                ),
                              );
                            } catch (error) {
                              setProviderError(errorText(error));
                            } finally {
                              setProviderBusy(false);
                            }
                          }}
                        >
                          {!chatgpt.activeAccountId && (
                            <option value="">Choose an account</option>
                          )}
                          {chatgpt.accounts.map((account) => (
                            <option key={account.id} value={account.id}>
                              {account.email
                                ? `${account.email} · ${account.label}`
                                : account.label}
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                    {chatgpt.connected && (
                      <label>
                        ChatGPT model
                        <select
                          aria-label="ChatGPT model"
                          value={state.settings.chatgptModel || ""}
                          disabled={generating || signingIn || providerBusy}
                          onChange={(event) =>
                            updateSettings({ chatgptModel: event.target.value })
                          }
                        >
                          {!chatgpt.models.length && (
                            <option value="">
                              Refresh to load available models
                            </option>
                          )}
                          {chatgpt.models.map((model) => (
                            <option key={model.slug} value={model.slug}>
                              {model.displayName}
                            </option>
                          ))}
                        </select>
                      </label>
                    )}
                    {chatgpt.connected && (
                      <p className="helper-line">
                        {chatgpt.planUsage
                          ? "Using ChatGPT plan · your account’s usage limits apply."
                          : "Plan usage is not enabled. Reconnect and approve it to answer in NextCue."}
                      </p>
                    )}
                    <div className="provider-actions">
                      {signingIn ? (
                        <>
                          <p className="helper-line">
                            <Loader2 size={14} className="spin" /> Finish
                            sign-in in your browser, then return here.
                          </p>
                          <button
                            className="button secondary"
                            onClick={() =>
                              void api
                                .cancelChatgptSignIn?.()
                                .catch((error) =>
                                  setProviderError(errorText(error)),
                                )
                            }
                          >
                            Cancel sign-in
                          </button>
                        </>
                      ) : (
                        <button
                          className="button primary"
                          disabled={generating || providerBusy}
                          onClick={() =>
                            void connectChatgpt(chatgpt.activeAccountId)
                          }
                        >
                          {chatgpt.connected
                            ? "Reconnect ChatGPT"
                            : "Continue with ChatGPT"}
                        </button>
                      )}
                      {!!chatgpt.accounts?.length && !signingIn && (
                        <button
                          className="text-button"
                          disabled={generating || providerBusy}
                          onClick={() => void connectChatgpt()}
                        >
                          Add account
                        </button>
                      )}
                      {chatgpt.connected && !signingIn && (
                        <>
                          <button
                            className="text-button"
                            onClick={() =>
                              void api
                                .openProviderPage?.("chatgpt-usage")
                                .catch((error) =>
                                  setProviderError(errorText(error)),
                                )
                            }
                          >
                            Manage usage
                          </button>
                          <button
                            className="text-button"
                            disabled={generating || providerBusy}
                            onClick={async () => {
                              if (!api.signOutChatgpt) return;
                              setProviderBusy(true);
                              try {
                                acceptChatgpt(await api.signOutChatgpt());
                              } catch (error) {
                                setProviderError(errorText(error));
                              } finally {
                                setProviderBusy(false);
                              }
                            }}
                          >
                            Disconnect
                          </button>
                        </>
                      )}
                    </div>
                    <p className="helper-line">
                      Available for eligible local projects and approved apps.
                      Your ChatGPT conversations are not imported.
                    </p>
                  </div>
                )}
                {answerProvider === "claude" && (
                  <div className="provider-connection">
                    <div className="provider-account">
                      <MessageCircle size={24} />
                      <div>
                        <h3>
                          {claude.connected && claude.subscription
                            ? "Claude connected"
                            : "Use your Claude Code login"}
                        </h3>
                        <p>
                          {claude.email ||
                            "NextCue runs your installed, unmodified Claude Code to answer here."}
                        </p>
                      </div>
                    </div>
                    {(!claude.connected || !claude.subscription) && (
                      <div className="connection-steps">
                        <p>
                          Install Claude Code, then sign in with your Claude
                          subscription through its own login.
                        </p>
                        <code>claude auth login</code>
                        <p>Return here and check the connection.</p>
                      </div>
                    )}
                    <p className="helper-line">
                      {claude.detail ||
                        (claude.available
                          ? "Claude Code is installed."
                          : "Claude Code was not found.")}
                    </p>
                    <button
                      className="text-button"
                      onClick={() =>
                        void api
                          .openProviderPage?.("claude-setup")
                          .catch((error) => setProviderError(errorText(error)))
                      }
                    >
                      Claude Code setup guide <ArrowRight size={13} />
                    </button>
                    <p className="helper-line">
                      Uses your Claude plan limits. NextCue does not collect
                      your Claude login or tokens.
                    </p>
                  </div>
                )}
                {answerProvider !== "local" && (
                  <>
                    <button
                      className="button secondary"
                      disabled={providerBusy || signingIn || generating}
                      onClick={() => void checkProviderConnection()}
                    >
                      <RefreshCw
                        size={14}
                        className={providerBusy ? "spin" : ""}
                      />
                      {providerBusy ? "Checking…" : "Check connection"}
                    </button>
                    {(providerError ||
                      (answerProvider === "chatgpt" && chatgpt.error)) && (
                      <p className="connection-error" role="alert">
                        {providerError || chatgpt.error}
                      </p>
                    )}
                  </>
                )}
              </section>
              <section className="panel settings-panel">
                <div className="panel-title">
                  <span className="setup-number">2</span>
                  <div>
                    <h2>Listening & audio</h2>
                    <p>Turn spoken questions into local text.</p>
                  </div>
                  <span
                    className={
                      "status-badge " +
                      (runtime.transcription ? "connected" : "neutral")
                    }
                  >
                    {runtime.transcription ? "Ready" : "Setup needed"}
                  </span>
                </div>
                {!runtime.transcription && (
                  <div className="setup-intro">
                    <AudioLines size={30} />
                    <h3>Give your assistant an ear.</h3>
                    <p>
                      Download the speech model once (148 MB). After setup, it
                      works offline.
                    </p>
                    <button
                      className="button primary"
                      disabled={voiceSetup}
                      onClick={() => void setupVoice()}
                    >
                      {voiceSetup ? (
                        <Loader2 size={16} className="spin" />
                      ) : (
                        <ArrowDownToLine size={16} />
                      )}
                      {voiceSetup ? "Setting up…" : "Set up transcription"}
                    </button>
                    {voiceSetup && (
                      <div className="progress-block">
                        <progress max="100" value={voiceProgress.progress} />
                        <span>
                          {voiceProgress.status}{" "}
                          {Math.round(voiceProgress.progress)}%
                        </span>
                      </div>
                    )}
                    {runtime.transcriptionError && (
                      <p className="connection-error">
                        {runtime.transcriptionError}
                      </p>
                    )}
                  </div>
                )}
                <label>
                  Input device
                  <select
                    value={state.settings.inputDevice}
                    onChange={(e) =>
                      updateSettings({
                        inputDevice: e.target.value,
                        audioSource:
                          e.target.value === "system" ? "system" : "microphone",
                      })
                    }
                  >
                    <option value="">Default microphone</option>
                    {devices.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>
                <button
                  className="text-button"
                  onClick={() =>
                    api
                      .inputDevices()
                      .then(setDevices)
                      .catch((e) => notify(errorText(e)))
                  }
                >
                  <RefreshCw size={13} />
                  Refresh devices
                </button>
                <label>
                  Spoken language
                  <select
                    value={state.settings.language}
                    onChange={(e) =>
                      updateSettings({ language: e.target.value })
                    }
                  >
                    <option value="en">English</option>
                    <option value="auto">Detect automatically</option>
                    <option value="es">Spanish</option>
                    <option value="fr">French</option>
                    <option value="de">German</option>
                    <option value="pt">Portuguese</option>
                  </select>
                </label>
                <p className="helper-line">
                  {runtime.platform === "macos"
                    ? "Choose Mac system audio to hear a remote caller through headphones. macOS may ask for Screen & System Audio Recording permission."
                    : "For call audio, route your computer's output to a virtual or loopback input, then choose it here."}
                </p>
              </section>
              <section className="panel settings-panel full">
                <div className="preference-row">
                  <div>
                    <strong>Suggest answers automatically</strong>
                    <p>
                      Generate a suggestion when a question is detected. Turn
                      this off to answer on click.
                    </p>
                  </div>
                  <button
                    className={
                      "toggle " + (state.settings.autoAnswer ? "on" : "")
                    }
                    role="switch"
                    aria-checked={state.settings.autoAnswer}
                    aria-label="Suggest answers automatically"
                    onClick={() =>
                      updateSettings({ autoAnswer: !state.settings.autoAnswer })
                    }
                  >
                    <span />
                  </button>
                </div>
                <div className="call-audio-note">
                  <ShieldCheck size={17} />
                  <p>
                    Listening starts only when you choose.{" "}
                    {answerProvider === "local" && !state.settings.jevEnabled
                      ? "Audio and answers stay on this computer."
                      : `Speech is transcribed locally. ${answerProvider !== "local" ? `Questions, background, and transcript context are sent to ${providerLabel} for answers.` : "Answers run locally."} ${state.settings.jevEnabled ? "Context is also sent to TypeSafe for optional checks." : ""}`}
                  </p>
                </div>
                <details className="context-helper-settings">
                  <summary>Optional context help with Jev</summary>
                  <p className="helper-line">
                    Jev checks whether more background or meeting notes may
                    help. It supports your chosen AI with context hints and does
                    not guarantee answer accuracy.
                  </p>
                  <div className="preference-row">
                    <div>
                      <strong>Use Jev context hints</strong>
                      <p>
                        When enabled, question context is sent to TypeSafe. This
                        service uses its own API key and usage allowance.
                      </p>
                    </div>
                    <button
                      className={
                        "toggle " + (state.settings.jevEnabled ? "on" : "")
                      }
                      role="switch"
                      aria-label="Use Jev context hints"
                      aria-checked={!!state.settings.jevEnabled}
                      disabled={
                        (!jevConfigured && !state.settings.jevEnabled) ||
                        jevBusy ||
                        generating
                      }
                      onClick={() =>
                        updateSettings({
                          jevEnabled: !state.settings.jevEnabled,
                        })
                      }
                    >
                      <span />
                    </button>
                  </div>
                  <form
                    onSubmit={(event) => {
                      event.preventDefault();
                      void saveContextHelper();
                    }}
                  >
                    <label>
                      TypeSafe API key
                      <input
                        type="password"
                        autoComplete="off"
                        spellCheck={false}
                        value={jevKey}
                        onChange={(event) => setJevKey(event.target.value)}
                        placeholder={
                          jevConfigured
                            ? "Key saved · enter a new key to replace it"
                            : "Enter your TypeSafe key"
                        }
                        maxLength={4096}
                        disabled={jevBusy || generating}
                      />
                    </label>
                    <div className="provider-actions">
                      <button
                        className="button secondary"
                        disabled={!jevKey.trim() || jevBusy || generating}
                      >
                        {jevBusy ? "Saving…" : "Save and enable context help"}
                      </button>
                      {jevConfigured && (
                        <button
                          type="button"
                          className="text-button"
                          disabled={jevBusy || generating}
                          onClick={async () => {
                            if (!api.deleteJevKey) return;
                            setJevBusy(true);
                            setJevError("");
                            try {
                              const result = await api.deleteJevKey();
                              setJevConfigured(result.configured);
                              updateSettings({ jevEnabled: false });
                            } catch (error) {
                              setJevError(errorText(error));
                            } finally {
                              setJevBusy(false);
                            }
                          }}
                        >
                          Remove key
                        </button>
                      )}
                    </div>
                  </form>
                  <p className="helper-line">
                    The key is stored in your operating system’s credential
                    store.
                  </p>
                  {jevError && (
                    <p className="connection-error" role="alert">
                      {jevError}
                    </p>
                  )}
                </details>
              </section>
              <section className="panel settings-panel full background-settings">
                <div className="panel-title">
                  <Monitor size={21} />
                  <div>
                    <h2>Background & meetings</h2>
                    <p>Keep NextCue ready in your system tray. Choose what a meeting trigger does.</p>
                  </div>
                </div>
                {([
                  ["runInBackground", "Keep running in system tray", "Closing the window keeps your session running. Quit NextCue from the tray menu to exit."],
                  ["startHidden", "Start in system tray", "Open quietly in the tray when you launch NextCue. Launching never starts listening by itself."],
                  ["meetingDetection", "Detect meeting hints", "Look for newly opened Zoom or Teams desktop apps and meeting pages reported by the browser companion."],
                ] as const).map(([key, label, description]) => (
                  <div className="preference-row" key={key}>
                    <div><strong>{label}</strong><p>{description}</p></div>
                    <button
                      className={"toggle " + (state.settings[key] ? "on" : "")}
                      role="switch"
                      aria-label={label}
                      aria-checked={state.settings[key] === true}
                      disabled={backgroundSaving || !api.configureBackground}
                      onClick={() => void saveBackgroundSettings({ [key]: !state.settings[key] })}
                    ><span /></button>
                  </div>
                ))}
                <label className="meeting-action-field">
                  <span>When a meeting is triggered</span>
                  <select
                    aria-label="When a meeting is triggered"
                    value={state.settings.meetingAction || "none"}
                    disabled={backgroundSaving || !api.configureBackground}
                    onChange={(event) => void saveBackgroundSettings({ meetingAction: event.target.value as BackgroundConfiguration["meetingAction"] })}
                  >
                    <option value="none">Do nothing</option>
                    <option value="prompt">Ask me first</option>
                    <option value="listen-notes">Listen and take notes</option>
                    <option value="listen-answer">Listen and answer</option>
                  </select>
                </label>
                <p className="helper-line">
                  This applies to the meeting shortcut and detected hints. Explicit tray actions work whenever you choose them.
                  App or page presence is a hint, not proof that a call has started. Listening continues until you pause it.
                </p>
                {state.settings.meetingAction?.startsWith("listen-") && (
                  <p className="background-status">
                    Listening starts automatically only for opted-in hints, or when you run the meeting trigger.
                    Use listening when meeting participants agree. Notes and answers use your selected AI provider.
                  </p>
                )}
                <details className="browser-connection">
                  <summary>Google Meet and browser meetings</summary>
                  <p>Pair the optional browser companion for Google Meet, Zoom web, Teams web, and supported meeting sites. It sends the provider name and a page-open hint, without page text or the meeting URL.</p>
                  <button
                    className="button secondary small-button"
                    disabled={!api.openBrowserCompanion}
                    onClick={() => void api.openBrowserCompanion?.().catch((e) => setBackgroundError(errorText(e)))}
                  ><FolderOpen size={14} />Open companion folder</button>{" "}
                  <button
                    className="button secondary small-button"
                    disabled={browserBusy || !api.getBrowserConnection}
                    onClick={async () => {
                      setBrowserBusy(true);
                      setBackgroundError("");
                      try { setBrowserConnection(await api.getBrowserConnection!()); }
                      catch (e) { setBackgroundError(errorText(e)); }
                      finally { setBrowserBusy(false); }
                    }}
                  >
                    {browserBusy ? <Loader2 size={14} className="spin" /> : <Plus size={14} />}
                    Connect browser companion
                  </button>
                  {browserConnection && (
                    browserConnection.available && browserConnection.port && browserConnection.token ? (
                      <div className="browser-pairing">
                        <label>
                          Browser connection code
                          <input
                            type="password"
                            autoComplete="off"
                            readOnly
                            value={JSON.stringify({ port: browserConnection.port, token: browserConnection.token })}
                          />
                        </label>
                        <button className="button secondary small-button" onClick={() => {
                          const code = JSON.stringify({ port: browserConnection.port, token: browserConnection.token });
                          void (api.copyText ? api.copyText(code) : navigator.clipboard.writeText(code))
                            .then(() => notify("Browser connection code copied."))
                            .catch((e) => setBackgroundError(errorText(e)));
                        }}><Copy size={14} />Copy connection code</button>
                        <p className="helper-line">Load the bundled browser-extension folder, open its options, and paste this code. Keep the code private. The browser companion must be installed separately.</p>
                      </div>
                    ) : <p className="connection-error">{browserConnection.detail || "The browser connection is unavailable."}</p>
                  )}
                </details>
                <p className="background-status">
                  {background.trayAvailable ? "System tray ready" : "System tray unavailable in this preview or desktop environment"}
                  {listening ? " · Listening" : " · Listening paused"}
                </p>
                {background.detail && <p className="helper-line">{background.detail}</p>}
                {backgroundError && <p className="connection-error" role="alert">{backgroundError}</p>}
              </section>
              <section className="panel settings-panel full assistant-settings">
                <div className="panel-title">
                  <EyeOff size={21} />
                  <div>
                    <h2>Assistant & screen sharing</h2>
                    <p>
                      Choose when your assistant is visible and how to call it
                      back.
                    </p>
                  </div>
                </div>
                <div className="preference-row">
                  <div>
                    <strong>
                      Exclude assistant from screen capture when supported
                    </strong>
                    <p>
                      {runtime.platform === "windows"
                        ? "Windows requests capture exclusion while NextCue stays visible to you. Support depends on the recording or sharing app; check its preview."
                        : "Capture exclusion is unavailable on this platform. Hide assistant removes NextCue from your desktop as well."}
                    </p>
                  </div>
                  <button
                    className={
                      "toggle " +
                      (state.settings.captureProtection !== false ? "on" : "")
                    }
                    role="switch"
                    aria-label="Exclude assistant from screen capture when supported"
                    aria-checked={state.settings.captureProtection !== false}
                    disabled={shortcutSaving || !api.configureCompanion}
                    onClick={() =>
                      void saveCompanionSettings({
                        captureProtection:
                          state.settings.captureProtection === false,
                      })
                    }
                  >
                    <span />
                  </button>
                </div>
                <div className="preference-row">
                  <div>
                    <strong>Allow clicks through floating assistant</strong>
                    <p>Pin and answer windows let you click the app underneath. Use {shortcut}, {visibilityShortcut}, or the system tray to recover the controls. The full app stays interactive.</p>
                  </div>
                  <button className={"toggle " + (state.settings.clickThrough ? "on" : "")} role="switch" aria-label="Allow clicks through floating assistant" aria-checked={state.settings.clickThrough === true} disabled={shortcutSaving || !api.configureCompanion || (companion.visibilityShortcutAvailable === false && !background.trayAvailable)} onClick={() => void saveCompanionSettings({ clickThrough: state.settings.clickThrough !== true })}><span /></button>
                </div>
                <p className="shortcut-status">For screen sharing, hide the assistant or choose Take notes in background in a meeting or video session. {runtime.platform === "macos" ? "A visible floating assistant can appear in a full-screen Zoom share on Mac." : "Check your sharing preview when using a visible floating assistant."} Clicks passing through affect interaction, not screen capture.</p>
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    void saveCompanionSettings(shortcutDraft);
                  }}
                >
                  <label className="shortcut-field">
                    <span>Show answer shortcut</span>
                    <input
                      value={shortcutDraft.answerShortcut}
                      maxLength={80}
                      onChange={(event) =>
                        setShortcutDraft((draft) => ({
                          ...draft,
                          answerShortcut: event.target.value,
                        }))
                      }
                      placeholder="CommandOrControl+Shift+Space"
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </label>
                  <label className="shortcut-field">
                    <span>Hide / show shortcut</span>
                    <input
                      value={shortcutDraft.visibilityShortcut}
                      maxLength={80}
                      onChange={(event) =>
                        setShortcutDraft((draft) => ({
                          ...draft,
                          visibilityShortcut: event.target.value,
                        }))
                      }
                      placeholder="CommandOrControl+Shift+H"
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </label>
                  <p className="shortcut-status">
                    Use keys such as <code>CommandOrControl+Alt+A</code>.
                    CommandOrControl uses Command on Mac and Ctrl on
                    Windows/Linux.
                  </p>
                  <label className="shortcut-field">
                    <span>Start meeting shortcut</span>
                    <input
                      value={shortcutDraft.meetingShortcut}
                      maxLength={80}
                      onChange={(event) => setShortcutDraft((draft) => ({ ...draft, meetingShortcut: event.target.value }))}
                      placeholder="CommandOrControl+Shift+M"
                      autoComplete="off"
                      spellCheck={false}
                    />
                  </label>
                  <div className="assistant-settings-actions">
                    <button
                      className="button secondary"
                      disabled={shortcutSaving || !api.configureCompanion}
                    >
                      {shortcutSaving ? (
                        <Loader2 size={15} className="spin" />
                      ) : (
                        <Check size={15} />
                      )}
                      Save shortcuts
                    </button>
                    <button
                      type="button"
                      className="button secondary"
                      disabled={
                        !api.setCompanionVisible ||
                        (companion.visibilityShortcutAvailable === false && !background.trayAvailable)
                      }
                      onClick={() => void hideAssistant()}
                    >
                      <EyeOff size={15} />
                      Hide assistant
                    </button>
                  </div>
                </form>
                {shortcutError && (
                  <p className="shortcut-status error" role="alert">
                    {shortcutError}
                  </p>
                )}
                <p className="shortcut-status">
                  Hide / show keeps listening and answers running. Press{" "}
                  <strong>{visibilityShortcut}</strong> to bring the assistant
                  back. The recovery shortcut{" "}
                  <strong>{emergencyShortcut}</strong> is always kept when
                  available.
                </p>
                <p className="shortcut-status">
                  Hide assistant hides the full app, pin, and answer card.
                  Earlier footage and a recorder’s cached frame may remain;
                  Hide cannot erase a recording or hide the system pointer.
                </p>
                <p className="shortcut-status">
                  In the full app, use Tab and Shift+Tab to move between
                  controls, Enter or Space to activate buttons, and{" "}
                  <strong>
                    {runtime.platform === "macos" ? "⌘" : "Ctrl"}+Enter
                  </strong>{" "}
                  to answer a question. Focus the answer to scroll with arrow
                  keys or Page Up / Page Down.
                </p>
              </section>
            </div>
          </div>
        ) : view === "live" && selected ? (
          <div className="live-page page">
            <div className="live-heading">
              <div>
                <button
                  className="text-button"
                  onClick={() => void navigate("history")}
                >
                  <ArrowLeft size={14} />
                  All sessions
                </button>
                <h1>{selected.title}</h1>
                <p>
                  {selected.role || selected.mode}
                  {selected.company ? " · " + selected.company : ""}
                </p>
              </div>
              <div className="live-heading-actions">
                <button
                  className="button secondary"
                  aria-label="Hide assistant"
                  title={"Hide for screen sharing · " + visibilityShortcut}
                  disabled={companion.visibilityShortcutAvailable === false && !background.trayAvailable}
                  onClick={() => void hideAssistant()}
                >
                  <EyeOff size={17} />
                  Hide for screen sharing
                </button>
                {(selected.mode === "Meeting" || selected.mode === "Video") && selected.status !== "ended" && <button className="button primary" disabled={backgroundNotesBusy || audioBusy || !api.setCompanionVisible || (companion.visibilityShortcutAvailable === false && !background.trayAvailable)} onClick={() => void takeNotesInBackground()}>{backgroundNotesBusy ? <Loader2 size={15} className="spin" /> : <FileText size={15} />}{backgroundNotesBusy ? "Starting background notes…" : "Take notes in background"}</button>}
                <button
                  className="button secondary"
                  disabled={companionBusy}
                  onClick={() => void setCompanionMode("pin")}
                  title="Shrink to a movable floating assistant"
                >
                  <Pin size={15} />
                  Pin mode
                </button>
                <details className="session-menu">
                  <summary className="icon-button" aria-label="Session options">
                    <MoreHorizontal size={20} />
                  </summary>
                  <div>
                    <button
                      disabled={audioBusy}
                      onClick={() => void exportMarkdown()}
                    >
                      <ArrowDownToLine size={15} />
                      Export Markdown
                    </button>
                    <button onClick={editContext}>
                      <FileText size={15} />
                      Edit background
                    </button>
                  </div>
                </details>
                {selected.status !== "ended" && (
                  <button
                    className="button end-button"
                    disabled={audioBusy}
                    onClick={() => void endSession()}
                  >
                    <Square size={12} />
                    End session
                  </button>
                )}
              </div>
            </div>
            {(selected.mode === "Meeting" || selected.mode === "Video") && <p className="background-notes-guide">Background notes hide NextCue while listening continues. Bring it back with <strong>{visibilityShortcut}</strong> or the system tray. {runtime.platform === "macos" && "Hide the floating assistant before sharing your full screen on Mac."}</p>}
            <div className="audio-toolbar">
              <div className={"listening-badge " + (listening ? "on" : "")}>
                <span className={"dot " + (listening ? "green" : "gray")} />
                {listening
                  ? "Listening"
                  : selected.status === "ended"
                    ? "Session ended"
                    : "Listening paused"}
              </div>
              {selected.mode === "Video" && runtime.platform === "macos" ? (
                <span className="video-audio-source"><Monitor size={14} />Mac system audio</span>
              ) : <label className="compact-select">
                <Mic size={14} />
                <select
                  aria-label="Audio input device"
                  value={selected.mode === "Video" ? videoInputs[selected.id] || "" : state.settings.inputDevice}
                  disabled={listening || audioBusy}
                  onChange={(e) => {
                    if (selected.mode === "Video") setVideoInputs((inputs) => ({ ...inputs, [selected.id]: e.target.value }));
                    updateSettings({
                      inputDevice: e.target.value,
                      audioSource:
                        e.target.value === "system" ? "system" : "microphone",
                    });
                  }}
                >
                  <option value="">{selected.mode === "Video" ? "Choose loopback input" : "Default microphone"}</option>
                  {devices.filter((device) => selected.mode !== "Video" || device.id !== "system").map((d) => (
                    <option key={d.id} value={d.id}>
                      {d.name}
                    </option>
                  ))}
                </select>
              </label>}
              {selected.status !== "ended" && audioButton}
              <span className="session-clock">
                <Clock3 size={14} />
                {duration(selected.duration)}
              </span>
            </div>
            <div className="session-toolbar">
              {selected.mode === "Video" ? (
                <div className="video-mode-picker" role="group" aria-label="Video assistance">
                  <button className={videoSubtitles ? "" : "selected"} aria-pressed={!videoSubtitles} onClick={() => void setVideoAssistance("notes")}><FileText size={15} />Notes</button>
                  <button className={videoSubtitles ? "selected" : ""} aria-pressed={videoSubtitles} onClick={() => void setVideoAssistance("subtitles")}><Captions size={15} />Subtitles</button>
                </div>
              ) : <div className="auto-answer">
                <Zap size={14} />
                <span>Auto suggestions</span>
                <button
                  className={
                    "toggle small-toggle " +
                    (autoAnswersEnabled(selected.id) ? "on" : "")
                  }
                  role="switch"
                  aria-checked={autoAnswersEnabled(selected.id)}
                  aria-label="Auto suggestions"
                  onClick={() => {
                    const enabled = !autoAnswersEnabled(selected.id);
                    updateSettings({ autoAnswer: enabled });
                    updateSession(selected.id, (s) => ({ ...s, assistanceMode: enabled ? "answers" : "manual" }));
                  }}
                >
                  <span />
                </button>
              </div>}
              <div className="session-details">
                <button
                  className={
                    "text-button " +
                    (detailsTab === "transcript" ? "selected" : "")
                  }
                  onClick={() =>
                    setDetailsTab(
                      detailsTab === "transcript" ? null : "transcript",
                    )
                  }
                  aria-expanded={detailsTab === "transcript"}
                >
                  <AudioLines size={15} />
                  Transcript{" "}
                  <span className="transcript-count">
                    {selected.transcript.length}
                  </span>
                </button>
                <button
                  className={
                    "text-button " +
                    (detailsTab === "context" ? "selected" : "")
                  }
                  onClick={() =>
                    setDetailsTab(detailsTab === "context" ? null : "context")
                  }
                  aria-expanded={detailsTab === "context"}
                >
                  <FileText size={15} />
                  Background
                </button>
              </div>
            </div>
            <div
              className={"live-layout " + (detailsTab ? "details-open" : "")}
            >
              {selected.mode === "Video" ? (
                <div className="video-workspace">
                  <section className="panel video-guide">
                    <div><Video size={19} /><strong>Capture notes and subtitles from browser videos</strong></div>
                    <p>{runtime.platform === "macos"
                      ? "Open YouTube or another video in your browser, choose Start listening, and play it. NextCue captures Mac system audio, including through headphones. Allow Screen & System Audio Recording if macOS asks."
                      : "Route your computer’s playback audio to a loopback or virtual input, choose that input above for this session, then Start listening and play YouTube or another browser video. Make sure the chosen input carries playback audio rather than a physical microphone."} No browser extension is needed.</p>
                    <p className="helper-line">Subtitle timing follows elapsed time from your first listening start, plus your video starting position ({selected.mediaOffsetSeconds || 0}s). Keep playback running continuously. After pausing, seeking, or replaying the video, start a new session with its current position to keep subtitles aligned.</p>
                  </section>
                  {!videoSubtitles && (
                    <section className="panel answer-panel video-notes-panel">
                      <div className="video-panel-heading"><h2><FileText size={17} />Video notes</h2><span className="ai-local">{providerLabel}</span></div>
                      <p className="context-insight" role="status">Notes update about once a minute as new speech arrives. {aiReady ? `Your selected ${providerLabel} creates the notes.` : "Connect your selected AI in Settings to generate notes. You can capture subtitles without an AI provider."}</p>
                      <div className="answer-content" tabIndex={0} role="region" aria-label="Video notes">{answerBody}</div>
                      {selected.summary && <div className="answer-footer"><button className="button secondary" disabled={generating || audioBusy} onClick={() => void generate(notesPrompt(selected), "summary")}><RefreshCw size={15} />Update video notes</button><button className="text-button" onClick={copyAnswer}><Copy size={14} />Copy notes</button></div>}
                    </section>
                  )}
                  <section className="panel video-captions-panel">
                    <div className="video-panel-heading"><h2><Captions size={17} />Live captions</h2><span className="ai-local">LOCAL TRANSCRIPTION</span></div>
                    {videoSubtitles && <p className="context-insight" role="status">Subtitles mode captures speech without generating AI notes or automatic answers. Captions appear after each speech segment is transcribed.</p>}
                    <div className="latest-caption" role="region" aria-label="Latest caption" aria-live="polite">
                      <span>Latest caption</span>
                      <p>{latestCaption?.text || "Start listening and play the video to capture spoken captions."}</p>
                    </div>
                    <div className="video-export-actions">
                      <button className="button secondary" disabled={!api.exportSubtitles || !canExportSubtitles || audioBusy} onClick={() => void exportSubtitles("srt")}><ArrowDownToLine size={15} />Export SRT</button>
                      <button className="button secondary" disabled={!api.exportSubtitles || !canExportSubtitles || audioBusy} onClick={() => void exportSubtitles("vtt")}><ArrowDownToLine size={15} />Export VTT</button>
                      <p>Exports pause listening and save timed speech captions. Review transcription accuracy before sharing.</p>
                    </div>
                    <div className="video-transcript-heading"><h3>Video transcript</h3><span>{videoTranscript.length} segments</span></div>
                    <div className="video-transcript" role="region" aria-label="Video transcript" tabIndex={0}>
                      {videoTranscript.length ? videoTranscript.map((entry) => <div className="video-transcript-entry" key={entry.id}><time>{entry.startMs !== undefined && selected.mediaStartedAt ? mediaTime(selected, entry.startMs) : time(entry.time)}</time><p>{entry.text}</p></div>) : <p className="video-transcript-empty">Spoken video audio will appear here as it is transcribed.</p>}
                    </div>
                  </section>
                </div>
              ) : <section className="panel answer-panel live-answer-main">
                {selected.assistanceMode === "notes" && (
                  <p className="context-insight" role="status">
                    Note taking is on. Notes update about once a minute as new transcript arrives; automatic answers are off.
                  </p>
                )}
                {selected.mode === "Meeting" &&
                  dismissedMeetingHint !== selected.id && (
                    <div className="meeting-hint">
                      <FileText size={18} />
                      <div>
                        <strong>
                          Keep the conversation and the next steps.
                        </strong>
                        <p>
                          {selected.transcript.length
                            ? "Create notes from the transcript, with decisions and action items."
                            : "Start listening when participants agree, or type notes below. NextCue can summarize them as you go."}
                        </p>
                      </div>
                      <button
                        className="text-button"
                        disabled={!selected.transcript.length || generating}
                        onClick={() =>
                          void generate("Create meeting notes.", "summary")
                        }
                      >
                        Create notes
                      </button>
                      <button
                        className="icon-button"
                        aria-label="Dismiss meeting suggestion"
                        onClick={() => setDismissedMeetingHint(selected.id)}
                      >
                        <X size={14} />
                      </button>
                    </div>
                  )}
                <div className="answer-tabs">
                  <div>
                    <button
                      className={answerTab === "answer" ? "selected" : ""}
                      onClick={() => setAnswerTab("answer")}
                    >
                      <Sparkles size={16} />
                      Answer
                    </button>
                    <button
                      className={answerTab === "summary" ? "selected" : ""}
                      onClick={() => setAnswerTab("summary")}
                    >
                      <FileText size={16} />
                      {selected.mode === "Meeting" ? "Notes" : "Recap"}
                    </button>
                  </div>
                  <span className="ai-local">
                    <ShieldCheck size={12} />
                    {answerProvider === "local"
                      ? "ON DEVICE"
                      : providerLabel.toUpperCase()}
                  </span>
                </div>
                {contextInsight && (
                  <p className="context-insight" role="status">
                    {contextInsight}
                  </p>
                )}
                <div
                  className="answer-content"
                  tabIndex={0}
                  role="region"
                  aria-label={answerRegionLabel}
                >
                  {answerBody}
                </div>
                <div className="answer-footer">
                  {answerTab === "answer" ? (
                    <>
                      <form
                        className="question-form"
                        onSubmit={(e) => {
                          e.preventDefault();
                          void generate(question || latestQuestion);
                        }}
                      >
                        <textarea
                          aria-label={
                            selected.mode === "Meeting"
                              ? "Meeting question"
                              : "Interview question"
                          }
                          placeholder="Ask a question, or let your assistant listen…"
                          value={question}
                          onChange={(e) => setQuestion(e.target.value)}
                          disabled={selected.status === "ended"}
                          rows={2}
                        />
                        {generating ? (
                          <button
                            type="button"
                            className="send-button"
                            aria-label="Stop generating"
                            onClick={() => void cancel()}
                          >
                            <Square size={15} />
                          </button>
                        ) : (
                          <button
                            className="send-button"
                            aria-label="Generate answer"
                            disabled={
                              selected.status === "ended" ||
                              (!question.trim() && !latestQuestion)
                            }
                          >
                            <ArrowRight size={19} />
                          </button>
                        )}
                      </form>
                      <div className="input-footer">
                        <span>
                          {runtime.platform === "macos" ? "⌘" : "Ctrl"} + Enter
                          to answer
                        </span>
                        {selected.answers.length > 0 && (
                          <div className="answer-pagination">
                            <button
                              aria-label="Previous answer"
                              disabled={
                                (answerIndex ?? selected.answers.length - 1) ===
                                  0 || generating
                              }
                              onClick={() =>
                                setAnswerIndex((i) =>
                                  Math.max(
                                    0,
                                    (i ?? selected.answers.length - 1) - 1,
                                  ),
                                )
                              }
                            >
                              <ArrowLeft size={14} />
                            </button>
                            <span>
                              {(answerIndex ?? selected.answers.length - 1) + 1}{" "}
                              / {selected.answers.length}
                            </span>
                            <button
                              aria-label="Next answer"
                              disabled={
                                (answerIndex ?? selected.answers.length - 1) ===
                                  selected.answers.length - 1 || generating
                              }
                              onClick={() =>
                                setAnswerIndex((i) =>
                                  Math.min(
                                    selected.answers.length - 1,
                                    (i ?? selected.answers.length - 1) + 1,
                                  ),
                                )
                              }
                            >
                              <ArrowRight size={14} />
                            </button>
                            <button
                              className="text-button"
                              onClick={copyAnswer}
                            >
                              <Copy size={13} />
                              Copy
                            </button>
                          </div>
                        )}
                      </div>
                    </>
                  ) : (
                    selected.summary && (
                      <button
                        className="button secondary"
                        disabled={generating}
                        onClick={() =>
                          void generate("Summarize this session.", "summary")
                        }
                      >
                        <RefreshCw size={15} />
                        {selected.mode === "Meeting"
                          ? "Update meeting notes"
                          : "Regenerate recap"}
                      </button>
                    )
                  )}
                </div>
              </section>}
              {detailsTab && (
                <aside className="panel details-drawer">
                  <div className="column-heading">
                    <h2>
                      {detailsTab === "transcript"
                        ? "Transcript"
                        : "Your background"}
                    </h2>
                    <button
                      className="icon-button"
                      aria-label="Close details"
                      onClick={() => setDetailsTab(null)}
                    >
                      <X size={16} />
                    </button>
                  </div>
                  {detailsTab === "transcript" ? (
                    <div className="transcript-list">
                      {selected.transcript.length ? (
                        selected.transcript.map((entry) => (
                          <div className="transcript-entry" key={entry.id}>
                            <div>
                              <span>
                                {entry.source === "typed"
                                  ? "Your question"
                                  : entry.source === "system"
                                    ? selected.mode === "Video" ? "Video audio" : "Call audio"
                                    : "Microphone"}
                              </span>
                              <time>{time(entry.time)}</time>
                            </div>
                            <p>{entry.text}</p>
                            {selected.mode !== "Video" && <button
                              className="transcript-answer"
                              disabled={
                                generating || selected.status === "ended"
                              }
                              onClick={() => void generate(entry.text)}
                            >
                              <Sparkles size={12} />
                              Suggest answer
                            </button>}
                          </div>
                        ))
                      ) : (
                        <div className="transcript-empty">
                          <AudioLines size={25} />
                          <p>Spoken and typed questions will appear here.</p>
                        </div>
                      )}
                      <div ref={transcriptEnd} />
                    </div>
                  ) : (
                    <div className="drawer-section">
                      <h3>{selected.role || "Your experience"}</h3>
                      <p className="context-copy">
                        {selected.context ||
                          "Add your experience or a job description to make answers more relevant."}
                      </p>
                      <button
                        className="button secondary"
                        onClick={editContext}
                      >
                        <FileText size={14} />
                        Edit background
                      </button>
                      {selected.instructions && (
                        <>
                          <h3>Answer guidance</h3>
                          <p>{selected.instructions}</p>
                        </>
                      )}
                    </div>
                  )}
                </aside>
              )}
            </div>
            <div className="workspace-footnote">
              <ShieldCheck size={13} />
              <span>Keep the useful parts. Make the answer your own.</span>
              <span>
                {captureLabel} · Hide / show: {visibilityShortcut}
              </span>
            </div>
          </div>
        ) : view === "history" ? (
          <div className="home-page page">
            <div className="page-heading">
              <div>
                <span className="eyebrow">PICK UP A CONVERSATION</span>
                <h1>Your sessions.</h1>
                <p>
                  Revisit answers, review a recap, or continue where you left
                  off.
                </p>
              </div>
              <button className="button primary" onClick={() => newSession()}>
                <Plus size={16} />
                New session
              </button>
            </div>
            <div className="table-toolbar">
              <div className="filter-tabs">
                {[
                  ["all", "All sessions"],
                  ["ready", "Ready"],
                  ["active", "In progress"],
                  ["ended", "Completed"],
                ].map(([value, label]) => (
                  <button
                    key={value}
                    className={filter === value ? "selected" : ""}
                    onClick={() => setFilter(value)}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <label className="search-box">
                <Search size={16} />
                <input
                  aria-label="Search sessions"
                  placeholder="Find a session…"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                />
              </label>
            </div>
            <div className="recent-list">
              {visibleSessions.length ? (
                visibleSessions.map(sessionRow)
              ) : (
                <div className="empty-state">
                  <History size={28} />
                  <h3>
                    {state.sessions.length
                      ? "No matching sessions."
                      : "Your conversations belong here."}
                  </h3>
                  <p>
                    {state.sessions.length
                      ? "Try another search or filter."
                      : "Start a session and come back to your answers anytime."}
                  </p>
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="home-page page">
            <section className="welcome-panel">
              <div className="home-heading">
                <img src="./logo.svg" alt="" className="welcome-mark" />
                <span className="feature-tag">
                  <Sparkles size={13} />A little help, on cue
                </span>
              </div>
              <h1>What are we working on?</h1>
              <p className="home-subtitle">
                Your own space to prepare, think, and find the right words.
              </p>
              <div className="home-actions">
                <button
                  className="button primary home-primary"
                  onClick={() => newSession()}
                >
                  <Plus size={17} />
                  New session
                  <ArrowRight size={16} />
                </button>
                {lastSession && (
                  <button
                    className="button secondary"
                    onClick={() => void openSession(lastSession)}
                  >
                    <Play size={15} />
                    Continue last session
                  </button>
                )}
              </div>
            </section>
            {(!aiReady || !runtime.transcription) && (
              <section className="setup-strip">
                <div>
                  <strong>Let's get your assistant ready.</strong>
                  <p>Choose your AI, then set up local listening.</p>
                </div>
                <div className="setup-step">
                  <span className={"dot " + (aiReady ? "green" : "amber")} />
                  {aiReady ? `${providerLabel} connected` : "Connect your AI"}
                </div>
                <div className="setup-step">
                  <span
                    className={
                      "dot " + (runtime.transcription ? "green" : "amber")
                    }
                  />
                  {runtime.transcription ? "Audio ready" : "Set up listening"}
                </div>
                <button
                  className="text-button"
                  onClick={() => void navigate("settings")}
                >
                  Finish setup
                  <ArrowRight size={14} />
                </button>
              </section>
            )}
            <div className="quick-starts">
              <button
                className="quick-start quick-start-video"
                onClick={() => newSession("Video")}
              >
                <div className="session-icon mode-video"><Video size={21} /></div>
                <span><strong>Video notes and subtitles</strong><small>Capture YouTube and browser video audio</small></span>
                <ArrowRight size={16} />
              </button>
              <button
                className="quick-start"
                onClick={() => newSession("Practice")}
              >
                <div className="session-icon mode-practice">
                  <Mic size={21} />
                </div>
                <span>
                  <strong>Practice out loud</strong>
                  <small>Build confidence before the call</small>
                </span>
                <ArrowRight size={16} />
              </button>
              <button
                className="quick-start"
                onClick={() => newSession("Meeting")}
              >
                <div className="session-icon mode-meeting">
                  <MessageCircle size={21} />
                </div>
                <span>
                  <strong>A helping hand in a meeting</strong>
                  <small>Keep up with questions and ideas</small>
                </span>
                <ArrowRight size={16} />
              </button>
            </div>
            <section>
              <div className="recent-header">
                <div>
                  <h2>Recent conversations</h2>
                  <p>Ready whenever you want to return.</p>
                </div>
                <button
                  className="text-button"
                  onClick={() => void navigate("history")}
                >
                  View all
                  <ArrowRight size={14} />
                </button>
              </div>
              <div className="recent-list">
                {recentSessions.length ? (
                  recentSessions.map(sessionRow)
                ) : (
                  <div className="empty-state">
                    <MessageCircle size={27} />
                    <h3>Your first conversation starts here.</h3>
                    <p>
                      Create a session, add a little context, and ask your first
                      question.
                    </p>
                  </div>
                )}
              </div>
            </section>
            <div className="home-footnote">
              <ShieldCheck size={14} />
              <span>
                {answerProvider === "local" && !state.settings.jevEnabled
                  ? "Local AI. Your conversations stay with you."
                  : `${providerLabel} powers answers.${state.settings.jevEnabled ? " TypeSafe provides context hints." : ""} Sessions are saved on your computer.`}
              </span>
              <button className="text-button" onClick={() => setModal("help")}>
                How it works
              </button>
            </div>
          </div>
        )}
      </main>

      {modal === "create" && (
        <Modal
          title="Start a conversation"
          subtitle="Choose what you need. Add context to personalize the results anytime."
          onClose={() => setModal(null)}
          wide
        >
          <form onSubmit={createSession}>
            <div className="mode-picker">
              {(["Interview", "Practice", "Meeting", "Video"] as SessionMode[]).map(
                (mode) => (
                  <button
                    type="button"
                    key={mode}
                    className={draft.mode === mode ? "selected" : ""}
                    aria-pressed={draft.mode === mode}
                    onClick={() => setDraft((d) => ({ ...d, mode }))}
                  >
                    {mode === "Video" ? (
                      <Video size={16} />
                    ) : mode === "Meeting" ? (
                      <MessageCircle size={16} />
                    ) : mode === "Practice" ? (
                      <Mic size={16} />
                    ) : (
                      <AudioLines size={16} />
                    )}
                    {mode}
                  </button>
                ),
              )}
            </div>
            <label>
              Session title
              <input
                autoFocus
                value={draft.title}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, title: e.target.value }))
                }
                placeholder={draft.mode === "Video" ? "e.g. Notes from a YouTube tutorial" : "e.g. Software engineer interview"}
                maxLength={150}
                required
              />
            </label>
            <label>
              {draft.mode === "Video" ? "Learning objective" : draft.mode === "Meeting" ? "Role or objective" : "Role"}
              <span className="optional">Optional</span>
              <input
                value={draft.role}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, role: e.target.value }))
                }
                placeholder={
                  draft.mode === "Video" ? "e.g. Learn the main concepts" : draft.mode === "Meeting"
                    ? "e.g. Plan the next release"
                    : "e.g. Software Engineer"
                }
                maxLength={150}
              />
            </label>
            {draft.mode === "Video" && <>
              <label>
                Video starting position (seconds)
                <span className="optional">Optional</span>
                <input type="number" min="0" max="359999" step="any" value={draft.mediaOffsetSeconds} onChange={(event) => setDraft((value) => ({ ...value, mediaOffsetSeconds: event.target.value }))} />
              </label>
              <p className="helper-line video-create-guide">Enter the video’s current playhead when you start listening (for example, 90 for 1:30). Play YouTube or another browser video while NextCue listens to {runtime.platform === "macos" ? "Mac system audio" : "a selected loopback or virtual audio input"}. No browser extension is needed. Keep playback continuous; create a new session after pausing or seeking to keep subtitles aligned.</p>
            </>}
            <details className="form-disclosure">
              <summary>
                <Sparkles size={15} />
                {draft.mode === "Video" ? "Personalize notes" : "Personalize answers"}<span>Optional</span>
                <ChevronDown size={15} />
              </summary>
              <label>
                Company
                <input
                  value={draft.company}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, company: e.target.value }))
                  }
                  placeholder="Where is the conversation happening?"
                  maxLength={100}
                />
              </label>
              <label htmlFor="background-context">
                {draft.mode === "Video" ? "Video background" : draft.mode === "Meeting"
                  ? "Meeting background"
                  : "Your background"}
                <button
                  type="button"
                  className="text-button label-action"
                  onClick={() => void importContext()}
                >
                  <Upload size={13} />
                  Import text
                </button>
                <textarea
                  id="background-context"
                  value={draft.context}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, context: e.target.value }))
                  }
                  placeholder={
                    draft.mode === "Video" ? "Video topic, terminology, or concepts to focus on…" : draft.mode === "Meeting"
                      ? "Agenda, project facts, decisions to make…"
                      : "Your experience, resume, or the job description…"
                  }
                  rows={4}
                  maxLength={30000}
                />
              </label>
              <label>
                {draft.mode === "Video" ? "Notes guidance" : "Answer guidance"}
                <input
                  value={draft.instructions}
                  onChange={(e) =>
                    setDraft((d) => ({ ...d, instructions: e.target.value }))
                  }
                  placeholder={draft.mode === "Video" ? "e.g. Summarize concepts and include important examples" : "e.g. Keep answers concise and use examples"}
                  maxLength={3000}
                />
              </label>
            </details>
            <div className="modal-footer">
              <span>
                <ShieldCheck size={13} />
                Saved on your computer
              </span>
              <button className="button primary">
                Create session
                <ArrowRight size={15} />
              </button>
            </div>
          </form>
        </Modal>
      )}
      {modal === "context" && selected && (
        <Modal
          title={selected.mode === "Video" ? "Personalize your notes" : "Personalize your answers"}
          subtitle={selected.mode === "Video" ? "Add the topic and details you want the notes to focus on." : "Give your assistant the experience and examples you want it to draw from."}
          onClose={() => setModal(null)}
          wide
        >
          <label htmlFor="background-context">
            {selected.mode === "Video" ? "Video background" : "Your background"}
            <button
              className="text-button label-action"
              onClick={() => void importContext()}
            >
              <Upload size={13} />
              Import text
            </button>
            <textarea
              id="background-context"
              rows={8}
              value={draft.context}
              maxLength={30000}
              onChange={(e) =>
                setDraft((d) => ({ ...d, context: e.target.value }))
              }
            />
          </label>
          <label>
            {selected.mode === "Video" ? "Notes guidance" : "Answer guidance"}
            <textarea
              rows={2}
              value={draft.instructions}
              maxLength={3000}
              onChange={(e) =>
                setDraft((d) => ({ ...d, instructions: e.target.value }))
              }
            />
          </label>
          <div className="modal-footer">
            <button
              className="text-button danger-text"
              onClick={() => setModal("delete")}
            >
              <Trash2 size={14} />
              Delete session
            </button>
            <button
              className="button primary"
              onClick={() => {
                const inputError = sessionInputError({
                  context: draft.context,
                  instructions: draft.instructions,
                });
                if (inputError) {
                  notify(inputError);
                  return;
                }
                updateSession(selected.id, (s) => ({
                  ...s,
                  context: draft.context,
                  instructions: draft.instructions,
                }));
                setModal(null);
                notify("Context saved.");
              }}
            >
              <Check size={15} />
              Save context
            </button>
          </div>
        </Modal>
      )}
      {modal === "delete" && selected && (
        <Modal
          title="Delete this session?"
          subtitle="This removes its transcript, answers, and context from local storage."
          onClose={() => setModal(null)}
        >
          <div className="modal-footer">
            <button className="button secondary" onClick={() => setModal(null)}>
              Keep session
            </button>
            <button
              className="button end-button"
              onClick={async () => {
                await stopAudio();
                await cancel();
                setState((s) => ({
                  ...s,
                  sessions: s.sessions.filter(
                    (item) => item.id !== selected.id,
                  ),
                }));
                setModal(null);
                void navigate("workspace");
              }}
            >
              <Trash2 size={15} />
              Delete session
            </button>
          </div>
        </Modal>
      )}
      {modal === "help" && (
        <Modal
          title="A little help, on cue."
          subtitle="Three simple steps to your first answer."
          onClose={() => setModal(null)}
        >
          <div className="help-step">
            <span className="setup-number">1</span>
            <div>
              <h3>Get ready</h3>
              <p>
                Open Settings to choose your AI and set up listening. You can
                also type questions.
              </p>
            </div>
          </div>
          <div className="help-step">
            <span className="setup-number">2</span>
            <div>
              <h3>Start a conversation</h3>
              <p>
                Choose interview, practice, or meeting. Add your background if
                you'd like more personal answers.
              </p>
            </div>
          </div>
          <div className="help-step">
            <span className="setup-number">3</span>
            <div>
              <h3>Listen, ask, and make it yours</h3>
              <p>
                Start listening or type a question. Use Pin mode for a small
                movable assistant beside your call.
              </p>
            </div>
          </div>
          <div className="call-audio-note">
            <Pin size={17} />
            <p>
              Click the pin to show an answer. The full app and pin may appear
              in screen recordings or sharing. Share a specific app window or
              use an unshared monitor when available.
            </p>
          </div>
          <div className="modal-footer">
            <button
              className="button primary"
              onClick={() => {
                setModal(null);
                void navigate("settings");
              }}
            >
              Open setup
              <ArrowRight size={15} />
            </button>
          </div>
        </Modal>
      )}
      {toast && (
        <div className="toast" role="status">
          <CheckCircle2 size={17} />
          {toast}
          <button
            aria-label="Dismiss notification"
            onClick={() => setToast("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      {!native && !window.desktop && (
        <div className="preview-banner">
          Interface preview · Open the desktop app for Ollama and audio
        </div>
      )}
    </div>
  );
}
