import { useState } from "react";

export function TaskCounter({ name }: { name: string }) {
  const [count, setCount] = useState(1);
  return (
    <section>
      <p>Welcome back, {name}!</p>
      <p data-testid="count">You have {count} open tasks</p>
      <button type="button" onClick={() => setCount(count + 1)}>
        Add a task
      </button>
    </section>
  );
}
