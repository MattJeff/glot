import { Suspense } from "react";
import { ForgotPassword } from "@/components/email-pages";

export default function Page() {
  return (
    <Suspense>
      <ForgotPassword />
    </Suspense>
  );
}
