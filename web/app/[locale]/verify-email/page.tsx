import { Suspense } from "react";
import { VerifyEmail } from "@/components/email-pages";

export default function Page() {
  return (
    <Suspense>
      <VerifyEmail />
    </Suspense>
  );
}
