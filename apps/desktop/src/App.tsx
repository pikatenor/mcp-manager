import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [endpoint, setEndpoint] = useState("http://127.0.0.1:8757/mcp");

  useEffect(() => {
    invoke<string>("aggregator_endpoint")
      .then(setEndpoint)
      .catch(() => {});
  }, []);

  return (
    <main className="container">
      <h1>MCP Manager</h1>
      <p>Aggregated MCP endpoint (Streamable HTTP):</p>
      <code>{endpoint}</code>
      <p className="hint">Closing this window hides the app to the menu bar.</p>
    </main>
  );
}

export default App;
