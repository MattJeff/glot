// ?fake — stand-in for the Chrome Translator API (browsers without it, screenshots). Not a real translation:
// it brackets every word so you can see what was sent to the model and what was protected.
globalThis.Translator = {
  availability: async () => "available",
  create: async () => ({
    translate: async (s) => s.replace(/[\p{L}']+/gu, (w) => `«${w}»`),
  }),
};
