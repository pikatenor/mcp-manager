import { FormEvent, useState } from "react";
import {
  AlertIcon,
  LockIcon,
  PlayIcon,
  PlusIcon,
  RefreshIcon,
  ServerIcon,
  StopIcon,
  TrashIcon,
} from "../icons";
import type {
  AddServerRequest,
  ServerState,
  ServerTool,
  ServerType,
} from "../types";
import { EmptyState, StatusBadge, Switch, TypeBadge } from "./ui";

const TYPE_OPTIONS: { value: ServerType; label: string }[] = [
  { value: "local", label: "Local stdio" },
  { value: "remote", label: "Remote SSE" },
  { value: "remote-streamable", label: "Streamable HTTP" },
];

function parseEnv(raw: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    env[trimmed.slice(0, eq)] = trimmed.slice(eq + 1);
  }
  return env;
}

export function ServersView({
  servers,
  toolsByServer,
  oauthByServer,
  loaded,
  onAdd,
  onStart,
  onStop,
  onDelete,
  onToggleTool,
  onOauth,
}: {
  servers: ServerState[];
  toolsByServer: Record<string, ServerTool[]>;
  oauthByServer: Record<string, boolean>;
  loaded: boolean;
  onAdd: (request: AddServerRequest) => Promise<void>;
  onStart: (id: string) => Promise<void>;
  onStop: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onToggleTool: (id: string, toolName: string, isPublic: boolean) => Promise<void>;
  onOauth: (id: string) => Promise<void>;
}) {
  const [formOpen, setFormOpen] = useState(false);
  const runningCount = servers.filter((s) => s.status === "running").length;

  return (
    <>
      <header className="content-header">
        <div>
          <h1 className="content-title">Servers</h1>
          <p className="content-subtitle">
            {servers.length === 0
              ? "Local stdio and remote MCP servers"
              : `${runningCount} of ${servers.length} running`}
          </p>
        </div>
        <div className="content-actions">
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => setFormOpen((open) => !open)}
          >
            <PlusIcon size={14} />
            {formOpen ? "Close" : "Add server"}
          </button>
        </div>
      </header>
      <div className="content-body">
        {formOpen && (
          <AddServerForm
            onCancel={() => setFormOpen(false)}
            onSubmit={async (request) => {
              await onAdd(request);
              setFormOpen(false);
            }}
          />
        )}

        {loaded && servers.length === 0 ? (
          <EmptyState
            icon={<ServerIcon size={28} />}
            title="No servers yet"
            note="Add a local stdio server or connect to a remote endpoint."
            action={
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => setFormOpen(true)}
              >
                <PlusIcon size={14} />
                Add server
              </button>
            }
          />
        ) : (
          servers.map((server) => (
            <ServerCard
              key={server.config.id}
              server={server}
              tools={toolsByServer[server.config.id] ?? []}
              oauthConnected={oauthByServer[server.config.id] ?? false}
              onStart={onStart}
              onStop={onStop}
              onDelete={onDelete}
              onToggleTool={onToggleTool}
              onOauth={onOauth}
            />
          ))
        )}
      </div>
    </>
  );
}

function AddServerForm({
  onSubmit,
  onCancel,
}: {
  onSubmit: (request: AddServerRequest) => Promise<void>;
  onCancel: () => void;
}) {
  const [name, setName] = useState("");
  const [type, setType] = useState<ServerType>("local");
  const [command, setCommand] = useState("npx");
  const [args, setArgs] = useState("-y @modelcontextprotocol/server-everything");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [envText, setEnvText] = useState("");
  const [bearer, setBearer] = useState("");
  const [autoStart, setAutoStart] = useState(true);
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setSubmitting(true);
    try {
      await onSubmit({
        name,
        server_type: type,
        command: type === "local" ? command : null,
        args: type === "local" ? args.split(/\s+/).filter(Boolean) : [],
        env: parseEnv(envText),
        remote_url: type === "local" ? null : remoteUrl,
        auto_start: autoStart,
        bearer: type !== "local" && bearer ? bearer : null,
      });
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form className="card add-form" onSubmit={handleSubmit}>
      <div className="add-form-grid">
        <div className="field">
          <label className="field-label" htmlFor="server-name">
            Name
          </label>
          <input
            id="server-name"
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
            placeholder="filesystem"
            autoFocus
          />
        </div>
        <div className="field">
          <span className="field-label">Type</span>
          <div className="segmented" role="radiogroup" aria-label="Server type">
            {TYPE_OPTIONS.map((option) => (
              <button
                key={option.value}
                type="button"
                role="radio"
                aria-checked={type === option.value}
                className={`segmented-item${type === option.value ? " active" : ""}`}
                onClick={() => setType(option.value)}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>

        {type === "local" ? (
          <>
            <div className="field">
              <label className="field-label" htmlFor="server-command">
                Command
              </label>
              <input
                id="server-command"
                value={command}
                onChange={(e) => setCommand(e.currentTarget.value)}
                placeholder="npx"
              />
            </div>
            <div className="field">
              <label className="field-label" htmlFor="server-args">
                Arguments
              </label>
              <input
                id="server-args"
                value={args}
                onChange={(e) => setArgs(e.currentTarget.value)}
                placeholder="-y @modelcontextprotocol/server-filesystem"
              />
            </div>
          </>
        ) : (
          <div className="field field-wide">
            <label className="field-label" htmlFor="server-url">
              Endpoint URL
            </label>
            <input
              id="server-url"
              value={remoteUrl}
              onChange={(e) => setRemoteUrl(e.currentTarget.value)}
              placeholder="https://example.com/mcp"
            />
          </div>
        )}

        <div className="field field-wide">
          <label className="field-label" htmlFor="server-env">
            Environment variables
          </label>
          <textarea
            id="server-env"
            value={envText}
            onChange={(e) => setEnvText(e.currentTarget.value)}
            placeholder={"API_KEY=value\nOTHER_TOKEN=value"}
            rows={3}
          />
        </div>

        {type !== "local" && (
          <div className="field field-wide">
            <label className="field-label" htmlFor="server-bearer">
              Bearer token (optional)
            </label>
            <input
              id="server-bearer"
              type="password"
              value={bearer}
              onChange={(e) => setBearer(e.currentTarget.value)}
              placeholder="Stored in the keychain, not the database"
            />
          </div>
        )}
      </div>

      <div className="add-form-footer">
        <Switch checked={autoStart} onChange={setAutoStart} label="Auto-start" />
        <span className="form-note">
          <LockIcon size={13} className="form-note-icon" />
          Secrets stay in the macOS keychain
        </span>
        <span className="spacer" />
        <button type="button" className="btn" onClick={onCancel}>
          Cancel
        </button>
        <button
          type="submit"
          className="btn btn-primary"
          disabled={submitting || !name.trim()}
        >
          Add server
        </button>
      </div>
    </form>
  );
}

function ServerCard({
  server,
  tools,
  oauthConnected,
  onStart,
  onStop,
  onDelete,
  onToggleTool,
  onOauth,
}: {
  server: ServerState;
  tools: ServerTool[];
  oauthConnected: boolean;
  onStart: (id: string) => Promise<void>;
  onStop: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onToggleTool: (id: string, toolName: string, isPublic: boolean) => Promise<void>;
  onOauth: (id: string) => Promise<void>;
}) {
  const { config, status, last_error: lastError } = server;
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const isRemote = config.server_type !== "local";
  const commandLine =
    config.command !== null
      ? [config.command, ...config.args].join(" ")
      : (config.remote_url ?? "");

  return (
    <div className="card server-card">
      <div className="server-head">
        <StatusBadge status={status} />
        <span className="server-name">{config.name}</span>
        <TypeBadge type={config.server_type} />
        <div className="server-actions">
          {isRemote && (
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => onOauth(config.id)}
            >
              <RefreshIcon size={13} />
              {oauthConnected ? "Re-auth" : "OAuth"}
            </button>
          )}
          {status === "running" ? (
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => onStop(config.id)}
            >
              <StopIcon size={12} />
              Stop
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-sm"
              onClick={() => onStart(config.id)}
            >
              <PlayIcon size={13} />
              Start
            </button>
          )}
          <button
            type="button"
            className={`btn btn-sm icon-btn${confirmingDelete ? " btn-danger" : ""}`}
            title={confirmingDelete ? "Click again to delete" : "Delete server"}
            onClick={() => {
              if (confirmingDelete) {
                onDelete(config.id);
              } else {
                setConfirmingDelete(true);
                setTimeout(() => setConfirmingDelete(false), 3000);
              }
            }}
          >
            {confirmingDelete ? "Delete?" : <TrashIcon size={14} />}
          </button>
        </div>
      </div>

      <div className="server-meta">
        <code>{commandLine}</code>
        {config.env_keys.length > 0 && (
          <span>· {config.env_keys.length} env</span>
        )}
        {config.auto_start && <span className="badge badge-accent">auto</span>}
      </div>

      {lastError && (
        <div className="server-error">
          <span className="server-error-icon">
            <AlertIcon size={13} />
          </span>
          {lastError}
        </div>
      )}

      {status === "running" && tools.length > 0 && (
        <div className="tools">
          <div className="tools-head">Tools · {tools.length}</div>
          {tools.map((tool) => (
            <div className="tool-row" key={tool.name}>
              <span className="tool-name" title={tool.name}>
                {tool.name}
              </span>
              <span
                className={`tool-visibility${tool.public ? " public" : ""}`}
              >
                {tool.public ? "public" : "hidden"}
              </span>
              <Switch
                checked={tool.public}
                onChange={(checked) =>
                  onToggleTool(config.id, tool.name, checked)
                }
                label=""
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
