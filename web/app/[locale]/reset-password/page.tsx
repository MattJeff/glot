import { Suspense } from "react";
import { ResetPassword } from "@/components/email-pages";

export default function Page() {
  return (
    <Suspense>
      <ResetPassword />
    </Suspense>
  );
}
