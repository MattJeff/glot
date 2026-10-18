import { AppShell } from "@/components/app-shell";

export default function Layout({ children }: LayoutProps<"/[locale]/app">) {
  return <AppShell>{children}</AppShell>;
}
