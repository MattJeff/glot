import { Header } from "./components/Header";
import { Hero } from "./components/Hero";
import { TaskCounter } from "./components/TaskCounter";
import { PricingCards } from "./components/PricingCards";
import { AboutText } from "./components/AboutText";

export default function App() {
  return (
    <>
      <Header />
      <main>
        <Hero />
        <TaskCounter name="Ada" />
        <PricingCards />
        <AboutText />
      </main>
    </>
  );
}
