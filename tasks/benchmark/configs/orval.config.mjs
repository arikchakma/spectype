export default {
  bench: {
    input: { target: process.env.BENCH_INPUT, parserOptions: { externalRefs: { allow: ["*"] } } },
    output: { target: process.env.BENCH_OUT + "/api.ts", client: "fetch", mode: "split" },
  },
};
