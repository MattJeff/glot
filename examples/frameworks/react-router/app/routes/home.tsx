import { TaskCounter } from "../components/TaskCounter";

export default function Home() {
  return (
    <main>
      <h1>Ship your projects faster</h1>
      <p>Plan, track and deliver work with your whole team.</p>
      <TaskCounter name="Ada" />
    </main>
  );
}
