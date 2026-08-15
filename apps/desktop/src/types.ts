export type TokenRecord = {
  id: string;
  client_name: string;
  token_hash: string;
  issued_at: number;
  revoked_at: number | null;
};

export type IssuedToken = {
  id: string;
  client_name: string;
  plaintext: string;
  issued_at: number;
};

export type ServerType = "local" | "remote" | "remote-streamable";

export type ServerConfig = {
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

export type ServerStatus =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "error";

export type ServerState = {
  config: ServerConfig;
  status: ServerStatus;
  last_error: string | null;
};

export type ServerTool = {
  name: string;
  public: boolean;
};

export type AddServerRequest = {
  name: string;
  server_type: ServerType;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  remote_url: string | null;
  auto_start: boolean;
  bearer: string | null;
};
