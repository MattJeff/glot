// « Connect GitHub » API (crates/server/src/github_app.rs). Same error envelope as lib/api.ts.
import { ApiError, type Job } from "@/lib/api";

export type GithubJob = Omit<Job, "kind"> & { kind: "github_localize" | "github_update" };
export type GithubLink = {
  installation_id: number;
  repo: string;
  branch: string;
  pr_number: number | null;
  pr_url: string | null;
  pr_branch: string | null;
  last_sha: string | null;
  updated_at: string;
};
export type GithubStatus = {
  /** null = the GitHub App is not set up on this server. */
  app: { slug: string; html_url: string } | null;
  installations: { id: number; account: string }[];
  link: GithubLink | null;
  job: GithubJob | null;
};
export type GithubRepo = { full_name: string; default_branch: string; private: boolean };
export type GithubAppStatus = {
  configured: boolean;
  can_admin: boolean;
  slug: string | null;
  app_id: number | null;
  html_url: string | null;
  webhook_url: string;
};

/** Job steps, in order: progress.done = steps finished (see STEPS in github_app.rs). */
export const STEPS = ["token", "clone", "localize", "sync", "push", "pr"] as const;

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(path, {
      method,
      credentials: "include",
      headers: body === undefined ? undefined : { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError(0, "network", "network error");
  }
  if (res.status === 204) return undefined as T;
  const data = await res.json().catch(() => null);
  if (!res.ok) throw new ApiError(res.status, data?.error?.code ?? "error", data?.error?.message ?? res.statusText);
  return data as T;
}

const p = (slug: string) => `/api/projects/${encodeURIComponent(slug)}/github`;
export const githubApi = {
  status: (slug: string) => call<GithubStatus>("GET", p(slug)),
  connect: (slug: string) => call<{ install_url: string; authorize_url: string }>("POST", `${p(slug)}/connect`),
  repos: (slug: string, installation: number) =>
    call<GithubRepo[]>("GET", `${p(slug)}/repos?installation_id=${installation}`),
  link: (slug: string, b: { installation_id: number; repo: string; branch?: string }) =>
    call<{ link: GithubLink; job: GithubJob }>("PUT", `${p(slug)}/link`, b),
  unlink: (slug: string) => call<void>("DELETE", `${p(slug)}/link`),
  run: (slug: string) => call<GithubJob>("POST", `${p(slug)}/run`),
  app: () => call<GithubAppStatus>("GET", "/api/github/app"),
  manifest: (b: { name?: string; org?: string }) =>
    call<{ action: string; manifest: string }>("POST", "/api/github/app/manifest", b),
};
export const ghKey = {
  status: (slug: string) => ["github", slug] as const,
  repos: (slug: string, installation: number) => ["github", slug, "repos", installation] as const,
  app: ["github-app"] as const,
};
