// No @vitejs/plugin-react needed: esbuild's automatic JSX runtime is enough for this fixture.
export default { esbuild: { jsx: "automatic" }, build: { manifest: true } };
