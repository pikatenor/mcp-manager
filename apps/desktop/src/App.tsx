import { FormEvent, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type TokenRecord = {
  id: string;
  client_name: string;
  token_hash: string;
  issued_at: number;
  revoked_at: number | null;
};

type IssuedToken = {
  id: string;
  client_name: string;
  plaintext: string;
  issued_at: number;
};

type ServerType = "local" | "remote" | "remote-streamable";

type ServerConfig = {
  id: string;
  name: string;
  server_type: ServerType;
  command: string | null;
  args: string[];
  env_keys: string[];
  remote_url: string | null;
  auto_start: boolean;
  disabled: boolean;
};

type ServerState = {
  config: ServerConfig;
  status: "stopped" | "starting" | "running" | "stopping" | "error";
  last_error: string | null;
};

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

function App() {
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8757/mcp");
  const [clientName, setClientName] = useState("cursor");
  const [tokens, setTokens] = useState<TokenRecord[]>([]);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [servers, setServers] = useState<ServerState[]>([]);
  const [serverName, setServerName] = useState("");
  const [serverType, setServerType] = useState<ServerType>("local");
  const [command, setCommand] = useState("npx");
  const [args, setArgs] = useState("-y @modelcontextprotocol/server-everything");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [envText, setEnvText] = useState("");
  const [bearer, setBearer] = useState("");
  const [autoStart, setAutoStart] = useState(true);
  const [error, setError] = useState<string | null>(null);

  async function refreshTokens() {
    const listed = await invoke<TokenRecord[]>("list_tokens");
    setTokens(listed);
  }

  async function refreshServers() {
    const listed = await invoke<ServerState[]>("list_servers");
    setServers(listed);
  }

  useEffect(() => {
    invoke<string>("aggregator_endpoint")
      .then(setEndpoint)
      .catch(() => {});
    refreshTokens().catch((err) => setError(String(err)));
    refreshServers().catch((err) => setError(String(err)));
  }, []);

  async function onIssue(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const issued = await invoke<IssuedToken>("issue_token", {
        clientName,
      });
      setPlaintext(issued.plaintext);
      await refreshTokens();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onRevoke(id: string) {
    setError(null);
    try {
      await invoke("revoke_token", { id });
      await refreshTokens();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onAddServer(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      await invoke("add_server", {
        request: {
          name: serverName,
          server_type: serverType,
          command: serverType === "local" ? command : null,
          args:
            serverType === "local"
              ? args.split(/\s+/).filter(Boolean)
              : [],
          env: parseEnv(envText),
          remote_url: serverType === "local" ? null : remoteUrl,
          auto_start: autoStart,
          bearer: bearer || null,
        },
      });
      setServerName("");
      setEnvText("");
      setBearer("");
      await refreshServers();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onStart(id: string) {
    setError(null);
    try {
      await invoke("start_server", { id });
      await refreshServers();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onStop(id: string) {
    setError(null);
    try {
      await invoke("stop_server", { id });
      await refreshServers();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onDelete(id: string) {
    setError(null);
    try {
      await invoke("delete_server", { id });
      await refreshServers();
    } catch (err) {
      setError(String(err));
    }
  }

  async function copyEndpoint() {
    await navigator.clipboard.writeText(endpoint);
  }

  return (
    <main className="container">
      <h1>MCP Manager</h1>
      <p>Aggregated MCP endpoint (Streamable HTTP):</p>
      <p>
        <code>{endpoint}</code>
        <button type="button" onClick={copyEndpoint}>
          Copy
        </button>
      </p>
      <p className="hint">Closing this window hides the app to the menu bar.</p>

      <h2>Servers</h2>
      <form className="stack" onSubmit={onAddServer}>
        <div className="row">
          <input
            value={serverName}
            onChange={(e) => setServerName(e.currentTarget.value)}
            placeholder="server name"
          />
          <select
            value={serverType}
            onChange={(e) => setServerType(e.currentTarget.value as ServerType)}
          >
            <option value="local">local stdio</option>
            <option value="remote">remote SSE</option>
            <option value="remote-streamable">remote Streamable HTTP</option>
          </select>
        </div>
        {serverType === "local" ? (
          <div className="row">
            <input
              value={command}
              onChange={(e) => setCommand(e.currentTarget.value)}
              placeholder="command"
            />
            <input
              value={args}
              onChange={(e) => setArgs(e.currentTarget.value)}
              placeholder="args"
            />
          </div>
        ) : (
          <input
            value={remoteUrl}
            onChange={(e) => setRemoteUrl(e.currentTarget.value)}
            placeholder="https://example.com/mcp"
          />
        )}
        <textarea
          value={envText}
          onChange={(e) => setEnvText(e.currentTarget.value)}
          placeholder={"ENV_NAME=value (one per line)"}
          rows={3}
        />
        {serverType !== "local" && (
          <input
            value={bearer}
            onChange={(e) => setBearer(e.currentTarget.value)}
            placeholder="optional bearer token (stored in keychain)"
          />
        )}
        <label className="hint">
          <input
            type="checkbox"
            checked={autoStart}
            onChange={(e) => setAutoStart(e.currentTarget.checked)}
          />{" "}
          auto-start
        </label>
        <button type="submit">Add server</button>
      </form>
      <ul>
        {servers.map((server) => (
          <li key={server.config.id}>
            <strong>{server.config.name}</strong>{" "}
            <span className="hint">
              {server.config.server_type} · {server.status}
            </span>
            {server.last_error && (
              <span className="hint"> · {server.last_error}</span>
            )}
            <div className="row">
              {server.status === "running" ? (
                <button type="button" onClick={() => onStop(server.config.id)}>
                  Stop
                </button>
              ) : (
                <button type="button" onClick={() => onStart(server.config.id)}>
                  Start
                </button>
              )}
              <button type="button" onClick={() => onDelete(server.config.id)}>
                Delete
              </button>
            </div>
          </li>
        ))}
      </ul>
      <p className="hint">
        Start/stop works now; upstream stdio and remote MCP transports are still
        placeholders, so aggregated tools stay empty until those land.
      </p>

      <h2>Client tokens</h2>
      <form className="row" onSubmit={onIssue}>
        <input
          value={clientName}
          onChange={(e) => setClientName(e.currentTarget.value)}
          placeholder="client name"
        />
        <button type="submit">Issue</button>
      </form>
      {plaintext && (
        <p>
          Copy this secret now; it will not be shown again:
          <br />
          <code>{plaintext}</code>
        </p>
      )}
      {error && <p className="hint">{error}</p>}
      <ul>
        {tokens.map((token) => (
          <li key={token.id}>
            {token.client_name}{" "}
            {token.revoked_at ? (
              <span className="hint">revoked</span>
            ) : (
              <button type="button" onClick={() => onRevoke(token.id)}>
                Revoke
              </button>
            )}
          </li>
        ))}
      </ul>
    </main>
  );
}

export default App;
