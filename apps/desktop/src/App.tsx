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

function App() {
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8757/mcp");
  const [clientName, setClientName] = useState("cursor");
  const [tokens, setTokens] = useState<TokenRecord[]>([]);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    const listed = await invoke<TokenRecord[]>("list_tokens");
    setTokens(listed);
  }

  useEffect(() => {
    invoke<string>("aggregator_endpoint")
      .then(setEndpoint)
      .catch(() => {});
    refresh().catch((err) => setError(String(err)));
  }, []);

  async function onIssue(event: FormEvent) {
    event.preventDefault();
    setError(null);
    try {
      const issued = await invoke<IssuedToken>("issue_token", {
        clientName,
      });
      setPlaintext(issued.plaintext);
      await refresh();
    } catch (err) {
      setError(String(err));
    }
  }

  async function onRevoke(id: string) {
    setError(null);
    try {
      await invoke("revoke_token", { id });
      await refresh();
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
