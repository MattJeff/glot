// A connector card: its credential fields, rendered by the generic form of the Integrations page.
// Labels and hints come from the `connectors.<kind>.fields.<name>` / `.hints.<name>` messages.
export type ConnectorField = {
  name: string;
  /** Stored encrypted on the server, never displayed again (input type password). */
  secret?: boolean;
  type?: "text" | "url" | "password";
  placeholder?: string;
  optional?: boolean;
};
export type ConnectorDef = {
  kind: string;
  /** Brand name, not translated. */
  title: string;
  fields: ConnectorField[];
  /** Anchor in /docs/integrations. */
  docs: string;
  /**
   * One-click connect (OAuth or the platform's own key authorization), offered when the server lists the kind.
   * `tenant` = what must be known before leaving for the platform: the store domain or the site address
   * (label: `connectors.tenant.<kind>`).
   */
  oneClick?: { tenant?: { placeholder: string } };
};
