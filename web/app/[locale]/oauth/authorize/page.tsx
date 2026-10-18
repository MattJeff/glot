import { Suspense } from "react";
import { Consent } from "./consent";

export default function Page() {
  return (
    <Suspense>
      <Consent />
    </Suspense>
  );
}
