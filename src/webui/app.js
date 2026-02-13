const I18N = {
  en: {
    ui_title: "nanobot-rs control ui",
    meta_boot_wait: "[BOOT] waiting for state...",
    meta_ok: "[OK] {time} :: version={version}",
    meta_err: "[ERR] {error}",
    btn_refresh: "[ REFRESH ]",
    btn_copy_status: "[ COPY STATUS ]",
    btn_send: "[ SEND ]",
    section_chat: "$ webchat",
    section_system: "$ system.status",
    section_health: "$ doctor.summary",
    section_channels: "$ channels.enabled",
    section_cron: "$ cron.jobs",
    section_sessions: "$ sessions.list",
    section_pairing: "$ pairing.pending",
    loading_text: "loading...",
    tip_doctor: "tips: nanobot-rs doctor --fix",
    tip_pairing: "tips: nanobot-rs pairing list",
    tip_cron: "tips: nanobot-rs cron list --all",
    empty: "(empty)",
    none: "none",
    unknown: "unknown",
    no_name: "(no name)",
    na: "n/a",
    key_model: "model",
    key_channels: "channels",
    key_sessions: "sessions",
    key_cron_jobs: "cron_jobs",
    key_pairing_pending: "pairing_pending",
    key_channel: "channel",
    key_next: "next",
    key_code: "code",
    key_requests: "requests",
    key_fix: "fix",
    tag_ok: "[OK]",
    tag_warn: "[WARN]",
    tag_fail: "[FAIL]",
    tag_enabled: "[ENABLED]",
    tag_disabled: "[DISABLED]",
    tag_session: "[SESSION]",
    check_config_file: "Config file",
    check_workspace_dir: "Workspace directory",
    check_workspace_files: "Workspace baseline files",
    check_provider_api: "Provider API credentials",
    check_agent_model: "Default model",
    check_channels_enabled: "Enabled channels",
    check_cron_jobs: "Scheduled jobs",
    hint_run_doctor_fix: "Run `nanobot-rs doctor --fix`.",
    chat_session_placeholder: "session key (e.g. webui:default)",
    chat_input_placeholder: "type message and press Enter...",
    role_user: "[YOU]",
    role_assistant: "[BOT]",
    role_error: "[ERR]",
    chat_error_prefix: "chat failed",
  },
  zh: {
    ui_title: "nanobot-rs 控制面板",
    meta_boot_wait: "[启动] 等待状态中...",
    meta_ok: "[正常] {time} :: 版本={version}",
    meta_err: "[错误] {error}",
    btn_refresh: "[ 刷新 ]",
    btn_copy_status: "[ 复制状态 ]",
    btn_send: "[ 发送 ]",
    section_chat: "$ 网页对话",
    section_system: "$ 系统状态",
    section_health: "$ 健康检查",
    section_channels: "$ 已启用渠道",
    section_cron: "$ 定时任务",
    section_sessions: "$ 会话列表",
    section_pairing: "$ 待配对请求",
    loading_text: "加载中...",
    tip_doctor: "提示: nanobot-rs doctor --fix",
    tip_pairing: "提示: nanobot-rs pairing list",
    tip_cron: "提示: nanobot-rs cron list --all",
    empty: "(空)",
    none: "无",
    unknown: "未知",
    no_name: "(未命名)",
    na: "无",
    key_model: "模型",
    key_channels: "渠道",
    key_sessions: "会话",
    key_cron_jobs: "定时任务",
    key_pairing_pending: "待配对",
    key_channel: "渠道",
    key_next: "下次",
    key_code: "验证码",
    key_requests: "请求数",
    key_fix: "修复",
    tag_ok: "[正常]",
    tag_warn: "[警告]",
    tag_fail: "[失败]",
    tag_enabled: "[已启用]",
    tag_disabled: "[已禁用]",
    tag_session: "[会话]",
    check_config_file: "配置文件",
    check_workspace_dir: "工作区目录",
    check_workspace_files: "工作区基础文件",
    check_provider_api: "Provider API 凭据",
    check_agent_model: "默认模型",
    check_channels_enabled: "已启用渠道",
    check_cron_jobs: "定时任务数量",
    hint_run_doctor_fix: "运行 `nanobot-rs doctor --fix`。",
    chat_session_placeholder: "会话键（例如 webui:default）",
    chat_input_placeholder: "输入消息后回车发送...",
    role_user: "[你]",
    role_assistant: "[助手]",
    role_error: "[错误]",
    chat_error_prefix: "对话失败",
  },
};

function detectLanguage() {
  const candidates =
    Array.isArray(navigator.languages) && navigator.languages.length > 0
      ? navigator.languages
      : [navigator.language || "en"];
  for (const lang of candidates) {
    if (/^zh\b/i.test(String(lang))) {
      return "zh";
    }
  }
  return "en";
}

let currentLang = detectLanguage();
if (!I18N[currentLang]) {
  currentLang = "en";
}

function t(key, vars) {
  const locale = I18N[currentLang] || I18N.en;
  let out = locale[key] || I18N.en[key] || key;
  if (vars && typeof out === "string") {
    Object.keys(vars).forEach((name) => {
      out = out.replaceAll(`{${name}}`, String(vars[name]));
    });
  }
  return out;
}

function text(el, value) {
  if (el) {
    el.textContent = value;
  }
}

function applyStaticTranslations() {
  document.documentElement.lang = currentLang === "zh" ? "zh-CN" : "en";

  const titleEl = document.querySelector("title[data-i18n]");
  if (titleEl) {
    titleEl.textContent = t(titleEl.getAttribute("data-i18n"));
  }

  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    if (!key) {
      return;
    }
    el.textContent = t(key);
  });

  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    const key = el.getAttribute("data-i18n-placeholder");
    if (!key) {
      return;
    }
    el.setAttribute("placeholder", t(key));
  });
}

function mapHealthLabel(check) {
  const mapping = {
    "config.file": "check_config_file",
    "workspace.dir": "check_workspace_dir",
    "workspace.files": "check_workspace_files",
    "provider.api": "check_provider_api",
    "agent.model": "check_agent_model",
    "channels.enabled": "check_channels_enabled",
    "cron.jobs": "check_cron_jobs",
  };
  const key = mapping[check.id];
  return key ? t(key) : check.label;
}

function mapHealthHint(hint) {
  if (!hint) {
    return "";
  }
  if (hint === "Run `nanobot-rs doctor --fix`.") {
    return t("hint_run_doctor_fix");
  }
  return hint;
}

async function fetchState() {
  const response = await fetch("/api/state", { cache: "no-store" });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response.json();
}

async function postChat(message, session) {
  const response = await fetch("/api/chat", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      message,
      session: session || "webui:default",
    }),
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok || !payload.ok) {
    throw new Error(payload.error || `HTTP ${response.status}`);
  }
  return payload.response || "";
}

function renderList(container, items, mapItem) {
  if (!container) {
    return;
  }
  container.innerHTML = "";
  if (!items || items.length === 0) {
    const empty = document.createElement("div");
    empty.className = "item";
    empty.textContent = t("empty");
    container.appendChild(empty);
    return;
  }
  items.forEach((entry) => {
    container.appendChild(mapItem(entry));
  });
}

function buildItem(title, sub, level) {
  const div = document.createElement("div");
  div.className = `item ${level || ""}`.trim();
  const titleEl = document.createElement("div");
  titleEl.className = "title";
  titleEl.textContent = title;
  const subEl = document.createElement("div");
  subEl.className = "sub";
  subEl.textContent = sub;
  div.appendChild(titleEl);
  div.appendChild(subEl);
  return div;
}

function levelToClass(level) {
  if (!level) {
    return "";
  }
  const lower = String(level).toLowerCase();
  if (lower === "fail") {
    return "fail";
  }
  if (lower === "warn") {
    return "warn";
  }
  return "";
}

function appendChatLine(role, content) {
  const container = document.getElementById("chat-log");
  if (!container) {
    return;
  }
  const line = document.createElement("div");
  line.className = `chat-line ${role}`;
  const prefix =
    role === "user"
      ? t("role_user")
      : role === "assistant"
        ? t("role_assistant")
        : t("role_error");
  line.textContent = `${prefix} ${content}`;
  container.appendChild(line);
  container.scrollTop = container.scrollHeight;
}

function renderState(state) {
  text(
    document.getElementById("meta-line"),
    t("meta_ok", { time: state.generatedAt, version: state.version })
  );
  text(
    document.getElementById("system-status"),
    [
      `${t("key_model")}=${state.model}`,
      `${t("key_channels")}=${(state.channelsEnabled || []).join(", ") || t("none")}`,
      `${t("key_sessions")}=${(state.sessions || []).length}`,
      `${t("key_cron_jobs")}=${(state.cronJobs || []).length}`,
      `${t("key_pairing_pending")}=${(state.pairingPending || []).length}`,
    ].join("\n")
  );

  const health = state.health || {};
  const summary = health.summary || { ok: 0, warn: 0, fail: 0 };
  const counters = document.getElementById("health-counters");
  if (counters) {
    counters.innerHTML = `
      <span class="counter ok">${t("tag_ok")} ${summary.ok}</span>
      <span class="counter warn">${t("tag_warn")} ${summary.warn}</span>
      <span class="counter fail">${t("tag_fail")} ${summary.fail}</span>
    `;
  }

  renderList(document.getElementById("health-checks"), health.checks || [], (check) =>
    buildItem(
      `${check.id} :: ${mapHealthLabel(check)}`,
      `${check.detail}${check.fix_hint ? ` | ${t("key_fix")}: ${mapHealthHint(check.fix_hint)}` : ""}`,
      levelToClass(check.level)
    )
  );

  renderList(document.getElementById("channels-list"), state.channelsEnabled || [], (channel) =>
    buildItem(`${t("key_channel")}=${channel}`, t("tag_enabled"))
  );

  renderList(document.getElementById("cron-list"), state.cronJobs || [], (job) =>
    buildItem(
      `${job.id || t("unknown")} :: ${job.name || t("no_name")}`,
      `${job.enabled ? t("tag_enabled") : t("tag_disabled")} ${t("key_next")}=${job.state?.nextRunAtMs || t("na")}`
    )
  );

  renderList(document.getElementById("sessions-list"), state.sessions || [], (key) =>
    buildItem(key, t("tag_session"))
  );

  renderList(document.getElementById("pairing-list"), state.pairingPending || [], (entry) =>
    buildItem(
      `${entry.channel}:${entry.sender_id || entry.senderId}`,
      `${t("key_code")}=${entry.code} ${t("key_requests")}=${entry.request_count || entry.requestCount || 1}`
    )
  );
}

async function refresh() {
  try {
    const state = await fetchState();
    renderState(state);
  } catch (err) {
    text(document.getElementById("meta-line"), t("meta_err", { error: String(err) }));
  }
}

async function sendChatMessage() {
  const sessionInput = document.getElementById("chat-session");
  const input = document.getElementById("chat-input");
  const sendButton = document.getElementById("chat-send-btn");
  if (!input || !sessionInput || !sendButton) {
    return;
  }
  const message = input.value.trim();
  if (!message) {
    return;
  }
  const session = sessionInput.value.trim() || "webui:default";
  appendChatLine("user", message);
  input.value = "";
  sendButton.disabled = true;
  try {
    const reply = await postChat(message, session);
    appendChatLine("assistant", reply);
  } catch (err) {
    appendChatLine("error", `${t("chat_error_prefix")}: ${String(err)}`);
  } finally {
    sendButton.disabled = false;
    input.focus();
  }
}

function setupActions() {
  const refreshBtn = document.getElementById("refresh-btn");
  if (refreshBtn) {
    refreshBtn.addEventListener("click", () => refresh());
  }
  const copyBtn = document.getElementById("copy-status-btn");
  if (copyBtn) {
    copyBtn.addEventListener("click", async () => {
      const status = document.getElementById("system-status")?.textContent || "";
      try {
        await navigator.clipboard.writeText(status);
      } catch (_) {
        // noop
      }
    });
  }
  const chatForm = document.getElementById("chat-form");
  if (chatForm) {
    chatForm.addEventListener("submit", (event) => {
      event.preventDefault();
      sendChatMessage();
    });
  }
}

applyStaticTranslations();
setupActions();
refresh();
setInterval(refresh, 5000);

