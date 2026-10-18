import { Suspense } from "react";
import { GitHubView } from "./view";

export default function Page() {
  return (
    <Suspense>
      <GitHubView />
    </Suspense>
  );
}
