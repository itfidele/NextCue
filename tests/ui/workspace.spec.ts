import { test, expect, type Page } from "@playwright/test";

async function bridge(page: Page) {
  await page.addInitScript(() => {
    type Callback = (event: any) => void;
    const events: Record<string, Set<Callback>> = {};
    let companionMode = "main";
    let companionVisible = true;
    let companionConfig = {
      captureProtection: true,
      answerShortcut: "CommandOrControl+Shift+Space",
      visibilityShortcut: "CommandOrControl+Shift+H",
    };
    const companionState = () => ({
      mode: companionMode,
      visible: companionVisible,
      captureProtection: companionConfig.captureProtection
        ? "unavailable"
        : "disabled",
      captureProtectionEnabled: companionConfig.captureProtection,
      shortcutAvailable: true,
      visibilityShortcutAvailable: true,
      answerShortcut: companionConfig.answerShortcut,
      visibilityShortcut: companionConfig.visibilityShortcut,
    });
    const emit = (name: string, event: any) => {
      if (name === "companion-visibility")
        companionVisible = typeof event === "boolean" ? event : event.visible;
      events[name]?.forEach((fn) => fn(event));
    };
    (window as any).__testEmit = emit;
    (window as any).__testRequests = [];
    (window as any).__testChatDelay = 400;
    (window as any).__testCompanionModes = [];
    (window as any).__testCancelledRequests = [];
    (window as any).__testStopListeningCalls = 0;
    (window as any).__testCompanionConfigurations = [];
    (window as any).__testCompanionVisibilityCalls = [];
    (window as any).__testCompanionStatusReads = 0;
    (window as any).__testConfigureError = false;
    (window as any).__testChatgpt = {
      connected: false,
      planUsage: false,
      models: [],
      accounts: [],
    };
    (window as any).__testClaude = {
      available: true,
      connected: false,
      subscription: false,
    };
    (window as any).__testPendingSignIn = false;
    (window as any).__testSignInCancels = 0;
    (window as any).__testSelectedAccounts = [];
    (window as any).__testChatgptRefreshRequests = [];
    (window as any).__testAssessments = [];
    (window as any).__testAssessmentError = false;
    (window as any).__testAssessment = {
      decision: "answer",
      confidence: 0.9,
      backgroundSupported: 0.9,
    };
    let rejectSignIn: ((error: Error) => void) | undefined;
    let jevKey = "";
    const on = (name: string, fn: Callback) => {
      (events[name] ||= new Set()).add(fn);
      return () => events[name].delete(fn);
    };
    const pending = new Map<string, { reject: (e: Error) => void }>();
    let activeSession = "";
    const blank = {
      sessions: [],
      settings: {
        model: "qwen2.5:7b-instruct",
        ollamaUrl: "http://127.0.0.1:11434",
        language: "en",
        autoAnswer: false,
        audioSource: "microphone",
        inputDevice: "",
      },
    };
    (window as any).desktop = {
      getState: async () =>
        JSON.parse(localStorage.getItem("test-state") || "null") || blank,
      saveState: async (state: any) =>
        localStorage.setItem("test-state", JSON.stringify(state)),
      status: async () => ({
        ollama: true,
        models: [{ name: "qwen2.5:7b-instruct", size: 4683000000 }],
        transcription: true,
        platform: "macos",
      }),
      getChatgptStatus: async (refreshModels = false) => {
        (window as any).__testChatgptRefreshRequests.push(refreshModels);
        return JSON.parse(JSON.stringify((window as any).__testChatgpt));
      },
      signInChatgpt: async () => {
        if ((window as any).__testPendingSignIn)
          return new Promise((_, reject) => {
            rejectSignIn = reject;
          });
        return (window as any).__testChatgpt;
      },
      cancelChatgptSignIn: async () => {
        (window as any).__testSignInCancels += 1;
        rejectSignIn?.(new Error("ChatGPT sign-in cancelled."));
        rejectSignIn = undefined;
      },
      signOutChatgpt: async () => {
        (window as any).__testChatgpt = {
          connected: false,
          planUsage: false,
          models: [],
          accounts: [],
        };
        return (window as any).__testChatgpt;
      },
      selectChatgptAccount: async (accountId: string) => {
        (window as any).__testSelectedAccounts.push(accountId);
        (window as any).__testChatgpt.activeAccountId = accountId;
        return (window as any).__testChatgpt;
      },
      getClaudeStatus: async () =>
        JSON.parse(JSON.stringify((window as any).__testClaude)),
      getJevStatus: async () => ({ configured: !!jevKey }),
      saveJevKey: async (key: string) => {
        jevKey = key;
        return { configured: true };
      },
      deleteJevKey: async () => {
        jevKey = "";
        return { configured: false };
      },
      assessContext: async (request: any) => {
        (window as any).__testAssessments.push(
          JSON.parse(JSON.stringify(request)),
        );
        if ((window as any).__testAssessmentError)
          throw new Error("Assessment unavailable.");
        return (window as any).__testAssessment;
      },
      startOllama: async () => {},
      chat: (request: any) =>
        new Promise((resolve, reject) => {
          (window as any).__testRequests.push(request);
          pending.set(request.requestId, { reject });
          setTimeout(
            () =>
              emit("token", {
                requestId: request.requestId,
                token: "A grounded ",
              }),
            30,
          );
          setTimeout(
            () => {
              emit("token", {
                requestId: request.requestId,
                token: "answer based on your background.",
              });
              if (pending.delete(request.requestId))
                resolve({
                  text: "A grounded answer based on your background.",
                  model: request.model,
                  durationMs: 400,
                });
            },
            (window as any).__testChatDelay,
          );
        }),
      cancelChat: async (id: string) => {
        (window as any).__testCancelledRequests.push(id);
        pending.get(id)?.reject(new Error("Cancelled"));
        pending.delete(id);
      },
      onToken: (fn: Callback) => on("token", fn),
      inputDevices: async () => [{ id: "system", name: "Mac system audio" }],
      startListening: async (request: any) => {
        activeSession = request.sessionId;
        (window as any).__testActiveSession = activeSession;
        emit("status", { sessionId: activeSession, listening: true });
      },
      stopListening: async () => {
        (window as any).__testStopListeningCalls += 1;
        if (!activeSession) return;
        emit("status", { sessionId: activeSession, listening: false });
        emit("transcript", {
          sessionId: activeSession,
          text: "The final audio segment.",
          source: "microphone",
          time: Date.now(),
        });
        activeSession = "";
      },
      onTranscript: (fn: Callback) => on("transcript", fn),
      onVoiceError: (fn: Callback) => on("error", fn),
      onVoiceStatus: (fn: Callback) => on("status", fn),
      onVoiceProgress: (fn: Callback) => on("progress", fn),
      setupVoice: async () => {},
      exportSession: async () => true,
      importContext: async () => null,
      setAlwaysOnTop: async () => {},
      setCompanionMode: async (mode: string) => {
        (window as any).__testCompanionModes.push(mode);
        companionMode = mode;
        return companionState();
      },
      onCompanionToggle: (fn: Callback) => on("companion-toggle", fn),
      configureCompanion: async (config: typeof companionConfig) => {
        (window as any).__testCompanionConfigurations.push(config);
        if ((window as any).__testConfigureError)
          throw new Error("Shortcut is already used by another app.");
        companionConfig = config;
        return companionState();
      },
      getCompanionStatus: async () => {
        (window as any).__testCompanionStatusReads += 1;
        return companionState();
      },
      setCompanionVisible: async (visible: boolean) => {
        (window as any).__testCompanionVisibilityCalls.push(visible);
        companionVisible = visible;
        return companionState();
      },
      onCompanionVisibility: (fn: Callback) =>
        on("companion-visibility", (event: any) =>
          fn(typeof event === "boolean" ? event : event.visible),
        ),
      minimize: () => {},
      onBeforeQuit: (fn: Callback) => on("quit", fn),
      quitApp: async (state: any) => {
        if (state) localStorage.setItem("test-state", JSON.stringify(state));
      },
    };
  });
}
async function create(
  page: Page,
  title: string,
  options: {
    mode?: string;
    role?: string;
    company?: string;
    context?: string;
  } = {},
) {
  await page.getByRole("button", { name: "New session", exact: true }).click();
  if (options.mode)
    await page
      .getByRole("dialog")
      .getByRole("button", { name: options.mode, exact: true })
      .click();
  await page.getByLabel("Session title").fill(title);
  await page
    .getByLabel("Role", { exact: false })
    .fill(options.role || "Software Engineer");
  await page
    .getByRole("dialog")
    .locator("summary")
    .filter({ hasText: "Personalize answers" })
    .click();
  if (options.company)
    await page.getByLabel("Company", { exact: true }).fill(options.company);
  await page
    .getByLabel(
      options.mode === "Meeting" ? "Meeting background" : "Your background",
    )
    .fill(options.context || "I built local developer tools in Rust.");
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Create session", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: title, exact: true }),
  ).toBeVisible();
}
async function heard(page: Page, text: string) {
  await page.evaluate((text) => {
    (window as any).__testEmit("transcript", {
      sessionId: (window as any).__testActiveSession,
      text,
      source: "microphone",
      time: Date.now(),
    });
  }, text);
}
async function chooseProvider(
  page: Page,
  provider: "chatgpt" | "claude",
  planUsage = true,
) {
  await page.evaluate(
    ({ provider, planUsage }) => {
      if (provider === "chatgpt")
        (window as any).__testChatgpt = {
          connected: true,
          planUsage,
          email: "person@example.com",
          activeAccountId: "account-a",
          accounts: [
            {
              id: "account-a",
              label: "Personal",
              email: "person@example.com",
              connected: true,
            },
          ],
          models: [
            { slug: "gpt-test-fast", displayName: "Fast test model" },
            { slug: "gpt-test-deep", displayName: "Detailed test model" },
          ],
        };
      else
        (window as any).__testClaude = {
          available: true,
          connected: true,
          subscription: true,
          email: "person@example.com",
        };
    },
    { provider, planUsage },
  );
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByLabel("Answer provider", { exact: true })
    .selectOption(provider);
  await page
    .getByRole("button", { name: "Check connection", exact: true })
    .click();
  await expect(
    page.getByText(
      provider === "chatgpt" ? "ChatGPT connected" : "Claude connected",
      { exact: true },
    ),
  ).toBeVisible();
}
async function enableContextHelper(page: Page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .locator("summary")
    .filter({ hasText: "Optional context help with Jev" })
    .click();
  await page
    .getByLabel("TypeSafe API key", { exact: true })
    .fill("synthetic-jeV-key-for-ui-test");
  await page
    .getByRole("button", { name: "Save and enable context help", exact: true })
    .click();
  await expect(
    page.getByRole("switch", { name: "Use Jev context hints", exact: true }),
  ).toHaveAttribute("aria-checked", "true");
  await expect(
    page.getByLabel("TypeSafe API key", { exact: true }),
  ).toHaveValue("");
}
test.beforeEach(async ({ page }) => {
  await bridge(page);
  await page.goto("/");
});

test("a session title that exceeds the UTF-8 limit stays editable before saving", async ({
  page,
}) => {
  await page.getByRole("button", { name: "New session", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const title = dialog.getByLabel("Session title", { exact: true });
  await title.fill("界".repeat(100));
  await dialog
    .getByRole("button", { name: "Create session", exact: true })
    .click();
  await expect(
    page.getByText("Shorten the session title before saving.", { exact: true }),
  ).toBeVisible();
  await expect(dialog).toBeVisible();
  await expect(title).toHaveValue("界".repeat(100));
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state") || "null")?.sessions
            ?.length || 0,
      ),
    )
    .toBe(0);
  await title.fill("日本語の面接");
  await dialog
    .getByRole("button", { name: "Create session", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "日本語の面接", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state") || "null")
            ?.sessions?.[0]?.title,
      ),
    )
    .toBe("日本語の面接");
});

test("a session can generate, save, end, and reopen its answer", async ({
  page,
}) => {
  await create(page, "Rust engineer interview");
  await page
    .getByLabel("Interview question")
    .fill("Tell me about your Rust experience.");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await page.getByRole("button", { name: "End session", exact: true }).click();
  await expect(page.getByText("Session ended", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "All sessions", exact: true }).click();
  await page.getByLabel("Search sessions").fill("Rust");
  await expect(page.locator(".session-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Review", exact: true }).click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await page.waitForTimeout(700);
  await page.reload();
  await page.getByRole("button", { name: "Review", exact: true }).click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
});
test("navigation cancels generation and late tokens cannot affect another session", async ({
  page,
}) => {
  await create(page, "Session A");
  await page.getByLabel("Interview question").fill("Why Rust?");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await page.getByRole("button", { name: "All sessions", exact: true }).click();
  await create(page, "Session B");
  await page.waitForTimeout(650);
  await expect(
    page.getByRole("heading", { name: "Find your words.", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".markdown")).toHaveCount(0);
  await page.getByLabel("Interview question").fill("A second question");
  await expect(
    page.getByRole("button", { name: "Generate answer", exact: true }),
  ).toBeEnabled();
});
test("stopping audio preserves its final transcript and the source choice", async ({
  page,
}) => {
  await create(page, "Voice practice");
  await page.getByLabel("Audio input device").selectOption("system");
  await page
    .getByRole("button", { name: "Start listening", exact: true })
    .click();
  await expect(page.getByText("Listening", { exact: true })).toBeVisible();
  await page
    .getByRole("button", { name: "Pause listening", exact: true })
    .click();
  await page.getByRole("button", { name: /^Transcript/ }).click();
  await expect(
    page.getByText("The final audio segment.", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Start listening", exact: true }),
  ).toBeEnabled();
  await page.waitForTimeout(650);
  const settings = await page.evaluate(
    () => JSON.parse(localStorage.getItem("test-state")!).settings,
  );
  expect(settings.audioSource).toBe("system");
  expect(settings.inputDevice).toBe("system");
  await page.screenshot({
    path: "/private/tmp/nextcue-session.png",
    fullPage: true,
  });
});
test("browser preview does not claim to generate real AI answers", async ({
  page,
}) => {
  await page.addInitScript(() => {
    delete (window as any).desktop;
  });
  await page.goto("/");
  await expect(
    page.getByText(
      "Interface preview · Open the desktop app for Ollama and audio",
    ),
  ).toBeVisible();
  await create(page, "Preview");
  await page.getByLabel("Interview question").fill("Test question");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "Start Ollama in Settings",
  );
  await expect(page.locator(".markdown")).toHaveCount(0);
});
test("a question heard during generation is queued and answered next", async ({
  page,
}) => {
  await create(page, "Queued questions");
  await page
    .getByRole("switch", { name: "Auto suggestions", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Start listening", exact: true })
    .click();
  await page.getByLabel("Interview question").fill("First question");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await page.evaluate(() => {
    const id = (window as any).__testActiveSession;
    (window as any).__testEmit("transcript", {
      sessionId: id,
      text: "How would you handle a second challenge?",
      source: "microphone",
      time: Date.now(),
    });
  });
  await expect(page.locator(".answered-question")).toHaveText(
    "How would you handle a second challenge?",
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__testRequests.length))
    .toBe(2);
});
test("recap includes early transcript entries and native quit flushes immediately", async ({
  page,
}) => {
  await create(page, "Full recap");
  await page.waitForTimeout(600);
  await page.evaluate(() => {
    const state = JSON.parse(localStorage.getItem("test-state")!);
    state.sessions[0].transcript = Array.from({ length: 30 }, (_, i) => ({
      id: String(i),
      text: `Topic ${i}`,
      source: "typed",
      time: Date.now() + i,
    }));
    localStorage.setItem("test-state", JSON.stringify(state));
  });
  await page.reload();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await page.getByRole("button", { name: "Recap", exact: true }).click();
  await page
    .getByRole("button", { name: "Generate recap", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText("A grounded answer");
  const request = await page.evaluate(() => (window as any).__testRequests[0]);
  expect(request.transcript).toContain("Topic 0\n");
  expect(request.transcript).toContain("Topic 29");
  await page.evaluate(() => (window as any).__testEmit("quit", null));
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state")!).sessions[0].summary,
      ),
    )
    .toBe("A grounded answer based on your background.");
});

test("Pin mode keeps listening and an in-flight answer alive when returning to the app", async ({
  page,
}) => {
  await create(page, "Floating answer");
  await page.evaluate(() => ((window as any).__testChatDelay = 1800));
  await page
    .getByRole("button", { name: "Start listening", exact: true })
    .click();
  const sessionId = await page.evaluate(
    () => (window as any).__testActiveSession,
  );
  const stopsBeforePin = await page.evaluate(
    () => (window as any).__testStopListeningCalls,
  );
  await page.getByLabel("Interview question").fill("How have you used Rust?");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await page.getByRole("button", { name: "Pin mode", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Show answer", exact: true }),
  ).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => (window as any).__testCompanionModes))
    .toEqual(["pin"]);
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(
    page.locator(".companion-card .markdown.streaming"),
  ).toContainText("A grounded");
  await expect(page.locator(".companion-state")).toContainText("Listening");
  await expect(
    page.getByRole("button", { name: "Pause listening", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".companion-card .markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await expect(page.locator(".answered-question")).toHaveText(
    "How have you used Rust?",
  );
  await page
    .getByRole("button", { name: "Open full app", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Floating answer", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Pause listening", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  const lifecycle = await page.evaluate(() => ({
    activeSession: (window as any).__testActiveSession,
    cancellations: (window as any).__testCancelledRequests,
    stops: (window as any).__testStopListeningCalls,
    requests: (window as any).__testRequests.length,
    modes: (window as any).__testCompanionModes,
  }));
  expect(lifecycle).toEqual({
    activeSession: sessionId,
    cancellations: [],
    stops: stopsBeforePin,
    requests: 1,
    modes: ["pin", "answer", "main"],
  });
});

test("clicking the pin answers the latest heard question with auto suggestions off", async ({
  page,
}) => {
  await create(page, "Click for help");
  await expect(
    page.getByRole("switch", { name: "Auto suggestions", exact: true }),
  ).toHaveAttribute("aria-checked", "false");
  await page
    .getByRole("button", { name: "Start listening", exact: true })
    .click();
  await heard(page, "An earlier question?");
  await heard(page, "What makes your Rust tools useful?");
  await page.getByRole("button", { name: "Pin mode", exact: true }).click();
  expect(await page.evaluate(() => (window as any).__testRequests.length)).toBe(
    0,
  );
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(page.locator(".answered-question")).toHaveText(
    "What makes your Rust tools useful?",
  );
  const request = await page.evaluate(() => (window as any).__testRequests[0]);
  expect(request.question).toBe("What makes your Rust tools useful?");
  expect(request.context).toContain("I built local developer tools in Rust.");
  expect(request.kind).toBe("answer");
  await expect(
    page.getByText("May appear in screen sharing", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Capture protection: best effort", { exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("button", { name: "Collapse answer", exact: true })
    .click();
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  expect(await page.evaluate(() => (window as any).__testRequests.length)).toBe(
    1,
  );
});

test("the native companion shortcut toggles answers and keeps the current session", async ({
  page,
}) => {
  await create(page, "Shortcut session");
  await page
    .getByLabel("Interview question")
    .fill("A draft question stays here");
  await page.evaluate(() =>
    (window as any).__testEmit("companion-toggle", null),
  );
  await expect(
    page.getByRole("button", { name: "Collapse answer", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as any).__testEmit("companion-toggle", null),
  );
  await expect(
    page.getByRole("button", { name: "Show answer", exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    (window as any).__testEmit("companion-toggle", null),
  );
  await expect(
    page.getByRole("button", { name: "Open full app", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Open full app", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Shortcut session", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Interview question")).toHaveValue(
    "A draft question stays here",
  );
  expect(
    await page.evaluate(() => (window as any).__testCompanionModes),
  ).toEqual(["answer", "pin", "answer", "main"]);
  expect(
    await page.evaluate(() => (window as any).__testCancelledRequests),
  ).toEqual([]);
});

test("a streaming recap survives Pin mode and remains a recap after completion", async ({
  page,
}) => {
  await create(page, "Floating recap");
  await page.getByLabel("Interview question").fill("Tell me about your tools.");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await page.evaluate(() => ((window as any).__testChatDelay = 1800));
  await page.getByRole("button", { name: "Recap", exact: true }).click();
  await page
    .getByRole("button", { name: "Generate recap", exact: true })
    .click();
  await page.getByRole("button", { name: "Pin mode", exact: true }).click();
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(
    page.locator(".companion-card .markdown.streaming"),
  ).toContainText("A grounded");
  await expect(page.getByText("SESSION RECAP", { exact: true })).toBeVisible();
  await expect(page.locator(".companion-card .markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  expect(
    await page.evaluate(() =>
      (window as any).__testRequests.map((request: any) => request.kind),
    ),
  ).toEqual(["answer", "summary"]);
  expect(
    await page.evaluate(() => (window as any).__testCancelledRequests),
  ).toEqual([]);
  await page
    .getByRole("button", { name: "Open full app", exact: true })
    .click();
  await expect(page.getByText("SESSION RECAP", { exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__testEmit("quit", null));
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state")!).sessions[0].summary,
      ),
    )
    .toBe("A grounded answer based on your background.");
});

test("custom assistant shortcuts and capture preferences apply and survive reload", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("heading", {
      name: "Assistant & screen sharing",
      exact: true,
    }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__testCompanionConfigurations.length),
    )
    .toBeGreaterThan(0);
  const configuredBeforeEditing = await page.evaluate(
    () => (window as any).__testCompanionConfigurations.length,
  );
  await page
    .getByLabel("Show answer shortcut", { exact: true })
    .fill("CommandOrControl+Alt+A");
  await page
    .getByLabel("Hide / show shortcut", { exact: true })
    .fill("CommandOrControl+Alt+H");
  expect(
    await page.evaluate(
      () => (window as any).__testCompanionConfigurations.length,
    ),
  ).toBe(configuredBeforeEditing);
  await page
    .getByLabel("Exclude assistant from screen capture when supported", {
      exact: true,
    })
    .click();
  await expect(
    page.getByRole("switch", {
      name: "Exclude assistant from screen capture when supported",
      exact: true,
    }),
  ).toHaveAttribute("aria-checked", "false");
  await page
    .getByRole("button", { name: "Save shortcuts", exact: true })
    .click();
  const configured = {
    captureProtection: false,
    answerShortcut: "CommandOrControl+Alt+A",
    visibilityShortcut: "CommandOrControl+Alt+H",
  };
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__testCompanionConfigurations.at(-1)),
    )
    .toEqual(configured);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const settings = JSON.parse(
          localStorage.getItem("test-state") || "null",
        )?.settings;
        if (!settings) return null;
        return {
          captureProtection: settings.captureProtection,
          answerShortcut: settings.answerShortcut,
          visibilityShortcut: settings.visibilityShortcut,
        };
      }),
    )
    .toEqual(configured);
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByLabel("Show answer shortcut", { exact: true }),
  ).toHaveValue(configured.answerShortcut);
  await expect(
    page.getByLabel("Hide / show shortcut", { exact: true }),
  ).toHaveValue(configured.visibilityShortcut);
  await page.screenshot({
    path: "/private/tmp/nextcue-settings.png",
    fullPage: true,
  });
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__testCompanionConfigurations.at(-1)),
    )
    .toEqual(configured);
});

test("a failed native shortcut configuration does not save unregistered bindings", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-state")))
    .not.toBeNull();
  const previousSettings = await page.evaluate(
    () => JSON.parse(localStorage.getItem("test-state")!).settings,
  );
  await page
    .getByLabel("Show answer shortcut", { exact: true })
    .fill("CommandOrControl+Alt+A");
  await page
    .getByLabel("Hide / show shortcut", { exact: true })
    .fill("CommandOrControl+Alt+H");
  await page.evaluate(() => ((window as any).__testConfigureError = true));
  await page
    .getByRole("button", { name: "Save shortcuts", exact: true })
    .click();
  await expect(page.getByRole("alert")).toContainText(
    "Shortcut is already used by another app.",
  );
  await page.waitForTimeout(650);
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("test-state")!).settings,
    ),
  ).toEqual(previousSettings);
  await page.evaluate(() => ((window as any).__testConfigureError = false));
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(
    page.getByLabel("Show answer shortcut", { exact: true }),
  ).toHaveValue(previousSettings.answerShortcut);
  await expect(
    page.getByLabel("Hide / show shortcut", { exact: true }),
  ).toHaveValue(previousSettings.visibilityShortcut);
});

test("hiding and showing the assistant preserves listening, an answer, and the current draft", async ({
  page,
}) => {
  await create(page, "Private screen sharing");
  await page.evaluate(() => ((window as any).__testChatDelay = 1800));
  await page
    .getByRole("button", { name: "Start listening", exact: true })
    .click();
  const before = await page.evaluate(() => ({
    session: (window as any).__testActiveSession,
    stops: (window as any).__testStopListeningCalls,
  }));
  await page.getByLabel("Interview question").fill("How have you used Rust?");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await page
    .getByLabel("Interview question")
    .fill("Keep this draft while hidden");
  await page.getByRole("button", { name: "Pin mode", exact: true }).click();
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(
    page.locator(".companion-card .markdown.streaming"),
  ).toContainText("A grounded");
  await page
    .getByRole("button", { name: "Hide assistant", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as any).__testCompanionVisibilityCalls),
    )
    .toEqual([false]);
  await page.evaluate(() =>
    (window as any).__testEmit("companion-visibility", { visible: true }),
  );
  await expect(page.locator(".companion-card .markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await page.evaluate(() =>
    (window as any).__testEmit("companion-visibility", { visible: false }),
  );
  await page.evaluate(() =>
    (window as any).__testEmit("companion-visibility", { visible: true }),
  );
  await expect(
    page.getByRole("button", { name: "Open full app", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Open full app", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Private screen sharing", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Interview question")).toHaveValue(
    "Keep this draft while hidden",
  );
  await expect(
    page.getByRole("button", { name: "Pause listening", exact: true }),
  ).toBeVisible();
  const after = await page.evaluate(() => ({
    session: (window as any).__testActiveSession,
    stops: (window as any).__testStopListeningCalls,
    cancellations: (window as any).__testCancelledRequests,
    requests: (window as any).__testRequests.length,
  }));
  expect(after).toEqual({ ...before, cancellations: [], requests: 1 });
});

test("a startup shortcut conflict restores accepted native settings and keeps the draft for editing", async ({
  page,
}) => {
  const requested = {
    captureProtection: false,
    answerShortcut: "CommandOrControl+Alt+A",
    visibilityShortcut: "CommandOrControl+Alt+H",
  };
  await page.evaluate(async (requested) => {
    const saved = await (window as any).desktop.getState();
    saved.settings = { ...saved.settings, ...requested };
    localStorage.setItem("test-state", JSON.stringify(saved));
  }, requested);
  await page.addInitScript(() => {
    (window as any).__testConfigureError = true;
  });
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Shortcut is already used by another app.",
  );
  await expect
    .poll(() => page.evaluate(() => (window as any).__testCompanionStatusReads))
    .toBe(1);
  expect(
    await page.evaluate(() =>
      (window as any).__testCompanionConfigurations.at(-1),
    ),
  ).toEqual(requested);
  await expect(
    page.getByRole("switch", {
      name: "Exclude assistant from screen capture when supported",
      exact: true,
    }),
  ).toHaveAttribute("aria-checked", "true");
  await expect(
    page.getByLabel("Show answer shortcut", { exact: true }),
  ).toHaveValue(requested.answerShortcut);
  await expect(
    page.getByLabel("Hide / show shortcut", { exact: true }),
  ).toHaveValue(requested.visibilityShortcut);
  await expect(
    page
      .locator(".assistant-settings .shortcut-status")
      .filter({ hasText: "Hide / show keeps" }),
  ).toContainText("⌘ + Shift + H");
  await expect(
    page
      .locator(".assistant-settings .shortcut-status")
      .filter({ hasText: "Hide / show keeps" }),
  ).not.toContainText("⌘ + Alt + H");
  await expect
    .poll(() =>
      page.evaluate(() => {
        const settings = JSON.parse(
          localStorage.getItem("test-state") || "null",
        )?.settings;
        if (!settings) return null;
        return {
          captureProtection: settings.captureProtection,
          answerShortcut: settings.answerShortcut,
          visibilityShortcut: settings.visibilityShortcut,
        };
      }),
    )
    .toEqual({
      captureProtection: true,
      answerShortcut: "CommandOrControl+Shift+Space",
      visibilityShortcut: "CommandOrControl+Shift+H",
    });
});

test("ChatGPT selection persists and sends the chosen model and grounded session context to the pin", async ({
  page,
}) => {
  await chooseProvider(page, "chatgpt");
  await page
    .getByLabel("ChatGPT model", { exact: true })
    .selectOption("gpt-test-deep");
  await page.setViewportSize({ width: 1160, height: 780 });
  await page.screenshot({
    path: "/private/tmp/nextcue-chatgpt-connected.png",
    fullPage: false,
  });
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await create(page, "Connected interview", { company: "Acme" });
  await page.getByLabel("Interview question").fill("How have you used Rust?");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  const request = await page.evaluate(() => (window as any).__testRequests[0]);
  expect(request).toMatchObject({
    provider: "chatgpt",
    model: "gpt-test-deep",
    sessionMode: "Interview",
    sessionTitle: "Connected interview",
    role: "Software Engineer",
    company: "Acme",
    context: "I built local developer tools in Rust.",
  });
  await page.getByRole("button", { name: "Pin mode", exact: true }).click();
  await page.getByRole("button", { name: "Show answer", exact: true }).click();
  await expect(page.locator(".companion-card .markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  await page
    .getByRole("button", { name: "Open full app", exact: true })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state") || "null")?.settings
            .answerProvider,
      ),
    )
    .toBe("chatgpt");
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByLabel("Answer provider", { exact: true })).toHaveValue(
    "chatgpt",
  );
});

test("ChatGPT without plan usage cannot silently generate an answer", async ({
  page,
}) => {
  await chooseProvider(page, "chatgpt", false);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await create(page, "Plan consent");
  await page
    .getByLabel("Interview question")
    .fill("Tell me about my experience.");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.getByRole("status")).toContainText(
    "Connect ChatGPT and enable plan usage in Settings first.",
  );
  expect(await page.evaluate(() => (window as any).__testRequests.length)).toBe(
    0,
  );
  await expect(page.locator(".markdown")).toHaveCount(0);
});

test("a connected Claude subscription uses its provider and returns answers inside the app", async ({
  page,
}) => {
  await chooseProvider(page, "claude");
  expect(
    await page.evaluate(() => (window as any).__testChatgptRefreshRequests),
  ).not.toContain(true);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await create(page, "Claude interview");
  await page
    .getByLabel("Interview question")
    .fill("How do I explain the tool I built?");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  expect(
    await page.evaluate(() => (window as any).__testRequests[0]),
  ).toMatchObject({
    provider: "claude",
    model: "default",
    sessionMode: "Interview",
    context: "I built local developer tools in Rust.",
  });
});

test("cancelling ChatGPT sign-in restores usable provider controls", async ({
  page,
}) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .getByLabel("Answer provider", { exact: true })
    .selectOption("chatgpt");
  await page.setViewportSize({ width: 1160, height: 780 });
  await expect(page.locator(".app-shell")).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Settings", exact: true }),
  ).toHaveClass(/active/);
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  await page.screenshot({
    path: "/private/tmp/nextcue-chatgpt-disconnected.png",
    fullPage: false,
    animations: "disabled",
  });
  await page.evaluate(() => ((window as any).__testPendingSignIn = true));
  await page
    .getByRole("button", { name: "Continue with ChatGPT", exact: true })
    .click();
  await expect(
    page.getByLabel("Answer provider", { exact: true }),
  ).toBeDisabled();
  await page
    .getByRole("button", { name: "Cancel sign-in", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Continue with ChatGPT", exact: true }),
  ).toBeEnabled();
  await expect(
    page.getByLabel("Answer provider", { exact: true }),
  ).toBeEnabled();
  expect(await page.evaluate(() => (window as any).__testSignInCancels)).toBe(
    1,
  );
  expect(await page.evaluate(() => (window as any).__testRequests.length)).toBe(
    0,
  );
});

test("Jev is off by default, keeps its key out of session storage, and adds optional context hints", async ({
  page,
}) => {
  await create(page, "Context guidance");
  await page.getByLabel("Interview question").fill("A first local question");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  expect(
    await page.evaluate(() => (window as any).__testAssessments.length),
  ).toBe(0);
  await enableContextHelper(page);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          JSON.parse(localStorage.getItem("test-state") || "null")?.settings
            .jevEnabled,
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(() => localStorage.getItem("test-state")),
  ).not.toContain("synthetic-jeV-key-for-ui-test");
  await page.evaluate(
    () =>
      ((window as any).__testAssessment = {
        decision: "clarify",
        confidence: 0.9,
        backgroundSupported: 0.1,
      }),
  );
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await page
    .getByLabel("Interview question")
    .fill("A question that needs more background");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".context-insight")).toContainText(
    "More context may help.",
  );
  await expect(page.locator(".answered-question")).toHaveText(
    "A question that needs more background",
  );
  expect(
    await page.evaluate(() => (window as any).__testRequests[1].instructions),
  ).toContain("Do not fill gaps with invented facts.");
  expect(
    await page.evaluate(() => (window as any).__testAssessments[0].context),
  ).toBe("I built local developer tools in Rust.");
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .locator("summary")
    .filter({ hasText: "Optional context help with Jev" })
    .click();
  await page.getByRole("button", { name: "Remove key", exact: true }).click();
  await expect(
    page.getByRole("switch", { name: "Use Jev context hints", exact: true }),
  ).toHaveAttribute("aria-checked", "false");
  await expect(
    page.getByRole("switch", { name: "Use Jev context hints", exact: true }),
  ).toBeDisabled();
  await page.evaluate(() => {
    const state = JSON.parse(localStorage.getItem("test-state")!);
    state.settings.jevEnabled = true;
    localStorage.setItem("test-state", JSON.stringify(state));
  });
  await page.reload();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page
    .locator("summary")
    .filter({ hasText: "Optional context help with Jev" })
    .click();
  const hints = page.getByRole("switch", {
    name: "Use Jev context hints",
    exact: true,
  });
  await expect(hints).toHaveAttribute("aria-checked", "true");
  await expect(hints).toBeEnabled();
  await hints.click();
  await expect(hints).toHaveAttribute("aria-checked", "false");
});

test("failed and uncertain Jev assessments leave the selected AI free to answer", async ({
  page,
}) => {
  await enableContextHelper(page);
  await page.getByRole("button", { name: "Home", exact: true }).click();
  await create(page, "Advisory context only");
  await page.evaluate(() => ((window as any).__testAssessmentError = true));
  await page
    .getByLabel("Interview question")
    .fill("Answer even if the context helper fails");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".answered-question")).toHaveText(
    "Answer even if the context helper fails",
  );
  await expect(page.locator(".context-insight")).toContainText(
    "The optional context helper is unavailable.",
  );
  await page.evaluate(() => {
    (window as any).__testAssessmentError = false;
    (window as any).__testAssessment = {
      decision: "clarify",
      confidence: 0.1,
      backgroundSupported: 0.0,
    };
  });
  await page
    .getByLabel("Interview question")
    .fill("Answer with an uncertain context assessment");
  await page
    .getByRole("button", { name: "Generate answer", exact: true })
    .click();
  await expect(page.locator(".answered-question")).toHaveText(
    "Answer with an uncertain context assessment",
  );
  await expect(page.locator(".context-insight")).toHaveCount(0);
  expect(
    await page.evaluate(() =>
      (window as any).__testRequests.map(
        (request: any) => request.instructions,
      ),
    ),
  ).toEqual(["", ""]);
  expect(
    await page.evaluate(() => (window as any).__testAssessments.length),
  ).toBe(2);
});

test("Meeting notes send full source-labelled context and meeting metadata at a desktop window size", async ({
  page,
}) => {
  await create(page, "Release planning", {
    mode: "Meeting",
    role: "Plan the release",
    company: "Acme",
    context: "Agenda: confirm the release date and next steps.",
  });
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("test-state")))
    .not.toBeNull();
  await page.evaluate(() => {
    const state = JSON.parse(localStorage.getItem("test-state")!);
    state.sessions[0].transcript = Array.from({ length: 30 }, (_, i) => ({
      id: String(i),
      text: `Release discussion ${i}`,
      source: i === 0 ? "system" : "microphone",
      time: Date.now() + i,
    }));
    localStorage.setItem("test-state", JSON.stringify(state));
  });
  await page.reload();
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("button", { name: "Open", exact: true }).click();
  await page.setViewportSize({ width: 1160, height: 780 });
  await expect(
    page.getByLabel("Meeting question", { exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Notes", exact: true }).click();
  await page
    .getByRole("button", { name: "Create meeting notes", exact: true })
    .click();
  await expect(page.locator(".markdown")).toContainText(
    "A grounded answer based on your background.",
  );
  const request = await page.evaluate(() => (window as any).__testRequests[0]);
  expect(request).toMatchObject({
    provider: "local",
    sessionMode: "Meeting",
    sessionTitle: "Release planning",
    role: "Plan the release",
    company: "Acme",
    kind: "summary",
    context: "Agenda: confirm the release date and next steps.",
  });
  expect(request.transcript).toContain("System audio: Release discussion 0\n");
  expect(request.transcript).toContain("Microphone: Release discussion 29");
  expect(request.instructions).not.toContain("STAR");
  await page.screenshot({
    path: "/private/tmp/nextcue-meeting-notes.png",
    fullPage: false,
  });
});
