import Link from "next/link";

export function Header() {
  return (
    <header>
      <strong>Acme Tasks</strong>
      <nav>
        <Link href="/">Home</Link> · <Link href="/pricing">Pricing</Link> · <Link href="/about">About</Link>
      </nav>
    </header>
  );
}
