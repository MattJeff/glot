import { Link } from "react-router";

export function Header() {
  return (
    <header>
      <strong>Acme Tasks</strong>
      <nav>
        <Link to="/">Home</Link> · <Link to="/pricing">Pricing</Link> · <Link to="/about">About</Link>
      </nav>
    </header>
  );
}
