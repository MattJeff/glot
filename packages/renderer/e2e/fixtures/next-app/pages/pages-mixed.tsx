export async function getStaticProps() { return { props: { title: "Legacy pricing page" } }; }
export default function P({ title }: { title: string }) {
  return <main><h1>{title}</h1><p>Cancel anytime</p></main>;
}
