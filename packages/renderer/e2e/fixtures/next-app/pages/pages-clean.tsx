export async function getStaticProps() { return { props: { title: "Legacy pricing page", cta: "Subscribe now" } }; }
export default function P({ title, cta }: { title: string; cta: string }) {
  return <main><h1>{title}</h1><button>{cta}</button></main>;
}
