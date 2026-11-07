"use client";
import { useState } from "react";
// label comes from the server component (RSC payload); no hard-coded text here
export function Cart({ label }: { label: string }) {
  const [n, setN] = useState(0);
  return <button onClick={() => setN(n + 1)}>{label}</button>;
}
// hard-coded text in the client bundle: the payload cannot carry its translation
export function Hard() {
  return <p>Cancel anytime</p>;
}
