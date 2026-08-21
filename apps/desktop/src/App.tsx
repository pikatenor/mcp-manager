import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { Sidebar, type View } from "./components/Sidebar";
import { ServersView } from "./components/ServersView";
import { TokensView } from "./components/TokensView";
import { Banner } from "./components/ui";
import type {
  AddServerRequest,
  IssuedToken,
  ServerState,
  ServerTool,
  TokenRecord,
} from "./types";

function App() {
  const [view, setView] = useState<View>("servers");
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8757/mcp");
  const [tokens, setTokens] = useState<TokenRecord[]>([]);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [servers, setServers] = useState<ServerState[]>([]);
  const [toolsByServer, setToolsByServer] = useState<
    Record<string, ServerTool[]>
  >({});
  const [oauthByServer, setOauthByServer] = useState<Record<string, boolean>>(
    {},
  );
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const runningIds = useRef<Set<string>>(new Set());

  async function refreshTokens() {
    const listed = await invoke<TokenRecord[]>("list_tokens");
    setTokens(listed);
  }

  async function fetchTools(
    running: ServerState[],
  ): Promise<Record<string, ServerTool[]>> {
    const entries = await Promise.all(
      running.map(async (server) => {
        const tools = await invoke<ServerTool[]>("list_server_tools", {
          id: server.config.id,
        });
        return [server.config.id, tools] as const;
      }),
    );
    return Object.fromEntries(entries);
  }

  async function refreshOauthFor(listed: ServerState[]) {
    const entries = await Promise.all(
      listed
        .filter((server) => server.config.server_type !== "local")
        .map(async (server) => {
          const connected = await invoke<boolean>("oauth_connected", {
            id: server.config.id,
          });
          return [server.config.id, connected] as const;
        }),
    );
    setOauthByServer(Object.fromEntries(entries));
  }

  const applyServerStates = useCallback((listed: ServerState[]) => {
    setServers(listed);

    // Tools change only when the set of running servers changes; fetching
    // them on every poll would hammer the aggregator for nothing.
    const nextRunning = new Set(
      listed
        .filter((server) => server.status === "running")
        .map((server) => server.config.id),
    );
    const changed =
      nextRunning.size !== runningIds.current.size ||
      [...nextRunning].some((id) => !runningIds.current.has(id));
    if (changed) {
      runningIds.current = nextRunning;
      const running = listed.filter((server) => server.status === "running");
      fetchTools(running)
        .then(setToolsByServer)
        .catch((err) => setError(String(err)));
    }
  }, []);

  const refreshServers = useCallback(async () => {
    const listed = await invoke<ServerState[]>("list_servers");
    applyServerStates(listed);
  }, [applyServerStates]);

  const refreshAll = useCallback(async () => {
    const listed = await invoke<ServerState[]>("list_servers");
    applyServerStates(listed);
    await refreshOauthFor(listed);
  }, [applyServerStates]);

  useEffect(() => {
    invoke<string>("aggregator_endpoint")
      .then(setEndpoint)
      .catch(() => {});
    refreshTokens().catch((err) => setError(String(err)));
    refreshAll()
      .catch((err) => setError(String(err)))
      .finally(() => setLoaded(true));

    // Statuses move on their own (starting → running/error), so poll the
    // cheap in-memory registry; heavy tool/oauth refreshes stay on demand.
    const poll = setInterval(() => {
      refreshServers().catch(() => {});
    }, 4000);
    return () => clearInterval(poll);
  }, [refreshAll, refreshServers]);

  async function onIssue(clientName: string) {
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

  async function onAddServer(request: AddServerRequest) {
    setError(null);
    try {
      await invoke("add_server", { request });
      await refreshAll();
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
      await refreshAll();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onToggleTool(id: string, toolName: string, isPublic: boolean) {
    setError(null);
    try {
      await invoke("set_tool_permission", { id, toolName, isPublic });
      const server = servers.find((s) => s.config.id === id);
      if (server?.status === "running") {
        const tools = await fetchTools([server]);
        setToolsByServer((prev) => ({ ...prev, ...tools }));
      }
    } catch (err) {
      setError(String(err));
    }
  }

  async function onOauth(id: string) {
    setError(null);
    try {
      await invoke("oauth_connect", { id });
      await refreshAll();
    } catch (err) {
      setError(String(err));
    }
  }

  return (
    <div className="app">
      <Sidebar
        view={view}
        onViewChange={setView}
        serverCount={servers.length}
        tokenCount={tokens.length}
        endpoint={endpoint}
      />
      <main className="content">
        {error && <Banner message={error} onDismiss={() => setError(null)} />}
        {view === "servers" ? (
          <ServersView
            servers={servers}
            toolsByServer={toolsByServer}
            oauthByServer={oauthByServer}
            loaded={loaded}
            onAdd={onAddServer}
            onStart={onStart}
            onStop={onStop}
            onDelete={onDelete}
            onToggleTool={onToggleTool}
            onOauth={onOauth}
          />
        ) : (
          <TokensView
            tokens={tokens}
            plaintext={plaintext}
            onIssue={onIssue}
            onRevoke={onRevoke}
            onDismissPlaintext={() => setPlaintext(null)}
          />
        )}
      </main>
    </div>
  );
}

export default App;
