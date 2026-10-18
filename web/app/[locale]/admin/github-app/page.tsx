import { Suspense } from "react";
import { GitHubAppSetup } from "./setup";

export default function Page() {
  return (
    <Suspense>
      <GitHubAppSetup />
    </Suspense>
  );
}
