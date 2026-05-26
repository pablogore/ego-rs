import type { Plugin } from "@opencode-ai/plugin";

const LOCAL = ["qwen", "deepseek", "gguf", "llama", "coder"];
const WRITE_TOOLS = new Set(["edit", "write"]);
const ALLOWED_TOOLS = new Set(["read", "edit", "write"]);
const SHELL_TOOLS = new Set(["bash", "shell"]);
const SAFE_GLOB = ["openspec/", "changes/"];
const PLANNER_PATTERNS = [
  "Thinking:", "let me inspect", "let me understand",
  "I will analyze", "next I will", "understand codebase",
  "explore architecture", "inspect repo", "working on task",
  "Which change would you like to work on",
  "## Goal", "## Constraints", "## Progress", "## Key Decisions",
  "## Next Steps", "## Critical Context", "## Relevant Files",
  "anchored summary", "update summary", "let me summarize", "next steps",
];
const BLOCKED_PATTERNS = [
  "no openspec changes directory was found",
  "tell me the name of the change",
  "run `openspec list`",
  "run openspec list",
];
const DISCOVERY_COMMANDS = ["git status", "git log", "find .opencode", "find openspec"];
const SPECULATIVE_IMPLEMENTATION_PATTERNS = [
  "HelloHandler",
  "CommandHandler",
  "QueryHandler",
  "impl HelloHandler",
  "fn handle(",
];
const PSEUDO_EXECUTION_PATTERNS = [
  "<tool_call",
  "</tool_call>",
  "<function=",
  "</function>",
  "<parameter=",
  "</parameter>",
  "<append",
  "</append>",
  "<write",
  "</write>",
  "<run",
  "</run>",
  "<filePath>",
  "</filePath>",
  "<command>",
  "</command>",
  "<content>",
  "</content>",
];
const PSEUDO_EXECUTION_REGEXES = [
  /<\s*\/?\s*tool_call\b/i,
  /<\s*function\s*=/i,
  /<\s*\/\s*function\s*>/i,
  /<\s*parameter\s*=/i,
  /<\s*\/\s*parameter\s*>/i,
  /<\s*\/?\s*(append|write|run|filePath|content|command)\b/i,
  /^\s*\{\s*"name"\s*:\s*"(skill|bash|read|write|edit|shell|apply_patch|glob)"\s*,\s*"arguments"\s*:/is,
  /^\s*\{\s*"tool"\s*:\s*"(skill|bash|read|write|edit|shell|apply_patch|glob)"\s*,/is,
  /^\s*\{\s*"function"\s*:\s*"(skill|bash|read|write|edit|shell|apply_patch|glob)"\s*,/is,
];

const normalizeResponse = (text: string): string =>
  text
    .toLowerCase()
    .replace(/\s+/g, " ")
    .replace(/[^\w\s:/.-]/g, "")
    .trim()
    .slice(0, 240);

const commandLooksLikeWrite = (command: string): boolean => {
  const lower = command.toLowerCase();
  return [
    "cat >",
    "cat <<",
    "tee ",
    "sed -i",
    "perl -0pi",
    "python",
    "mkdir ",
    "touch ",
    "install ",
  ].some((pattern) => lower.includes(pattern));
};

const commandLooksLikeTaskUpdate = (command: string): boolean => {
  const lower = command.toLowerCase();
  return (
    lower.includes("tasks.md") &&
    (lower.includes("- [x]") ||
      lower.includes("\\[x\\]") ||
      lower.includes("sed -i") ||
      lower.includes("perl -0pi") ||
      lower.includes("python"))
  );
};

export default (async function executionGuard(ctx) {
  let mode: "LOCAL" | "FRONTIER" = "LOCAL";
  let detected = false;
  let lastTool = "";
  let lastPath = "";
  let repeatCount = 0;
  let allowedPaths = new Set<string>();
  let responseHistory: string[] = [];
  let lastBlockedResponse = "";
  let blockedRepeatCount = 0;
  let activeChange = "";
  let readCount = 0;
  let readAllow = new Set<string>();
  let realWriteCount = 0;
  let realArtifactWriteCount = 0;
  let realTaskUpdateCount = 0;

  const abs = (p: string): string =>
    p.startsWith("/") ? p : ctx.repoRoot ? ctx.repoRoot + "/" + p.replace(/^\.\.\//g, "") : p;

  return {
    "experimental.chat.system.transform": async (input, _output) => {
      if (!detected && input.model?.modelID) {
        const id = input.model.modelID.toLowerCase();
        mode = LOCAL.some((p) => id.includes(p)) ? "LOCAL" : "FRONTIER";
        detected = true;
      }
      lastTool = "";
      lastPath = "";
      repeatCount = 0;
      readCount = 0;
      readAllow = new Set<string>();
      allowedPaths = new Set<string>();
      realWriteCount = 0;
      realArtifactWriteCount = 0;
      realTaskUpdateCount = 0;

      for (const p of (input as any).instructions || []) {
        if (typeof p === "string") allowedPaths.add(abs(p));
      }
      for (const f of (input as any).contextFiles || []) {
        allowedPaths.add(abs(typeof f === "string" ? f : f.path));
      }
      for (const f of (input as any).files || []) {
        allowedPaths.add(abs(f.path || f));
      }
    },

    "experimental.session.compacting": async (_input, output) => {
      if (mode !== "LOCAL") return;
      output.prompt = `EXECUTION MODE.
No planning. No summaries. No narration.
Execute the next required action using only real tool calls exposed by this runtime.
In OpenCode, use the real bash tool for filesystem mutations when write/edit tools are not exposed.
Never print pseudo-tool XML, JSON tool-call stubs, placeholder tool calls, or simulated write/edit/run blocks.
If no real bash/write/edit tool is available for the required action, return exactly FAIL_CLOSED: real_tool_unavailable.
Do not print NEXT_EXECUTABLE_ACTION.`;
    },

    "experimental.compaction.autocontinue": async (_input, output) => {
      if (mode === "LOCAL") output.enabled = false;
    },

    "tool.execute.after": async (input, output) => {
      if (mode !== "LOCAL") return;
      const tool = input.tool as string;
      const args = (input as any).args || {};
      const path = abs((args.path || args.filePath || "") as string);
      const command = (args.command || args.cmd || "") as string;
      if (
        tool === "edit" &&
        path.endsWith("/tasks.md") &&
        typeof args.oldString === "string" &&
        typeof args.newString === "string" &&
        args.oldString.includes("- [ ]") &&
        args.newString.includes("- [x]") &&
        !(output as any).error
      ) {
        realTaskUpdateCount++;
        return;
      }
      if (WRITE_TOOLS.has(tool) && !(output as any).error) {
        realWriteCount++;
        if (
          path.includes("/openspec/changes/") &&
          (path.endsWith("/proposal.md") ||
            path.endsWith("/design.md") ||
            path.endsWith("/tasks.md") ||
            path.endsWith("/.openspec.yaml") ||
            path.endsWith("/spec.md"))
        ) {
          realArtifactWriteCount++;
        }
      }
      if (SHELL_TOOLS.has(tool) && !(output as any).error) {
        if (commandLooksLikeWrite(command)) realWriteCount++;
        if (commandLooksLikeTaskUpdate(command)) realTaskUpdateCount++;
      }
      if (tool === "read" && path.endsWith("/tasks.md")) {
        const text = JSON.stringify(output).toLowerCase();
        if (text.includes("confirm spec-000 references") || text.includes("confirm governance references")) {
          readAllow.add("/specs/project-constitution/spec.md");
          readAllow.add("/specs/architecture-governance/spec.md");
          readAllow.add("/specs/testing-governance/spec.md");
        }
      }
    },

    "tool.execute.before": async (input, output) => {
      if (mode !== "LOCAL") return;

      const tool = input.tool as string;
      const args = (input as any).args || {};
      const path = abs((args.path || args.filePath || "") as string);
      const command = ((args.command || args.cmd || "") as string).toLowerCase();

      if (command.includes("openspec status --change")) {
        const name = command.split("--change")[1]?.trim().replace(/^["']|["']$/g, "").split(/\s+/)[0];
        if (name && !activeChange) activeChange = name;
      }
      if (activeChange && (DISCOVERY_COMMANDS.some((c) => command.includes(c)) || path.endsWith("/README.md"))) {
        output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: context_discovery_leakage" };
        return;
      }

      if (activeChange && WRITE_TOOLS.has(tool)) {
        const writeText = `${args.content || ""}\n${args.newString || ""}`;
        if (path.includes("/.opencode/") || path.endsWith("/README.md")) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: unrelated_repository_mutation" };
          return;
        }
        if (path.includes("/openspec/changes/archive/")) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: unrelated_repository_mutation" };
          return;
        }
        if (path.includes("/openspec/changes/") && !path.includes(`/openspec/changes/${activeChange}/tasks.md`)) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: scope_violation_detected" };
          return;
        }
        if (SPECULATIVE_IMPLEMENTATION_PATTERNS.some((p) => writeText.includes(p))) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: speculative_implementation_detected" };
          return;
        }
      }

      if (activeChange && SHELL_TOOLS.has(tool)) {
        if (command.includes(".opencode/") || command.includes("readme.md") || command.includes("openspec/changes/archive/")) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: unrelated_repository_mutation" };
          return;
        }
        if (command.includes("openspec/changes/") && !command.includes(`openspec/changes/${activeChange}/`)) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: scope_violation_detected" };
          return;
        }
        if (SPECULATIVE_IMPLEMENTATION_PATTERNS.some((p) => command.includes(p.toLowerCase()))) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: speculative_implementation_detected" };
          return;
        }
      }

      const marker = "/openspec/changes/";
      const idx = path.indexOf(marker);
      if (idx >= 0) {
        const rest = path.slice(idx + marker.length);
        const name = rest.startsWith("archive/") ? rest.slice(8).split("/")[0] : rest.split("/")[0];
        if (name && name !== "archive") {
          if (!activeChange) activeChange = name;
          if (activeChange && name !== activeChange) {
            output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: stale_context_resurrection" };
            return;
          }
        }
      }

      if (tool === "glob") {
        const pattern = (args.pattern || "") as string;
        if (!SAFE_GLOB.some((g) => pattern.startsWith(g))) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: repository_discovery_forbidden" };
          return;
        }
      }

      if (ALLOWED_TOOLS.has(tool)) {
        if (
          tool === "read" &&
          !path.endsWith("/tasks.md") &&
          !Array.from(readAllow).some((p) => path.endsWith(p))
        ) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: context_overfetch" };
          return;
        }
        if (!allowedPaths.has(path)) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: context_scope_violation" };
          return;
        }
        if (tool === "read" && ++readCount > 3) {
          output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: context_overfetch" };
          return;
        }
        if (tool === lastTool && path === lastPath) {
          repeatCount++;
          if (repeatCount > 2) {
            output.args = { ...output.args, _blocked: true, _reason: "FAIL_CLOSED: repeated_tool_execution" };
            return;
          }
        } else {
          repeatCount = 1;
        }
        lastTool = tool;
        lastPath = path;
      }
    },

    "experimental.text.complete": async (input, output) => {
      if (mode !== "LOCAL") return;
      const text = output.text || "";
      const lower = text.toLowerCase();
      const normalized = normalizeResponse(text);

      if (
        PSEUDO_EXECUTION_PATTERNS.some((p) => lower.includes(p.toLowerCase())) ||
        PSEUDO_EXECUTION_REGEXES.some((pattern) => pattern.test(text))
      ) {
        output.text = "FAIL_CLOSED: pseudo_execution_detected";
        return;
      }

      if (lower.includes("stopped_after_change_complete") || lower.includes("stopped_after_task_complete")) {
        if (realWriteCount === 0 || realTaskUpdateCount === 0) {
          output.text = "FAIL_CLOSED: task_state_not_persisted";
          return;
        }
      }

      if (lower.includes("status: apply-ready") || lower.includes("run `/opsx-apply`")) {
        if (realWriteCount === 0 || realArtifactWriteCount === 0) {
          output.text = "FAIL_CLOSED: artifact_state_not_persisted";
          return;
        }
      }

      if (activeChange) {
        if (lower.includes("current state:") || lower.includes("want me to continue")) {
          output.text = "FAIL_CLOSED: context_discovery_leakage";
          return;
        }
        const using = "using change:";
        const idx = lower.indexOf(using);
        if (idx >= 0) {
          const name = lower.slice(idx + using.length).trim().split(/\s+/)[0];
          if (name && name !== activeChange.toLowerCase()) {
            output.text = "FAIL_CLOSED: stale_context_resurrection";
            return;
          }
        }
        if (activeChange !== "project-governance" && lower.includes("project-governance")) {
          output.text = "FAIL_CLOSED: stale_context_resurrection";
          return;
        }
      }

      if (PLANNER_PATTERNS.some((p) => lower.includes(p.toLowerCase()))) {
        output.text = "FAIL_CLOSED: planner_leakage_detected";
        return;
      }

      if (BLOCKED_PATTERNS.some((p) => lower.includes(p))) {
        if (normalized === lastBlockedResponse) {
          blockedRepeatCount++;
        } else {
          lastBlockedResponse = normalized;
          blockedRepeatCount = 1;
        }
        if (blockedRepeatCount > 1) {
          output.text = "FAIL_CLOSED: repeated_blocked_response";
          blockedRepeatCount = 0;
          lastBlockedResponse = "";
          return;
        }
      }

      responseHistory.push(normalized);
      if (responseHistory.length > 6) responseHistory.shift();
      if (responseHistory.length >= 3) {
        const last = responseHistory.slice(-3);
        if (last[0] === last[1] && last[1] === last[2]) {
          output.text = "FAIL_CLOSED: semantic_loop_detected";
          responseHistory = [];
        }
      }
    },
  };
}) satisfies Plugin;
