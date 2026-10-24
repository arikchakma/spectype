import { defineConfig } from 'kubb'
import pluginTs from '@kubb/plugin-ts'

export default defineConfig({
  root: process.env.BENCH_OUT,
  input: process.env.BENCH_INPUT,
  output: { path: '.' },
  plugins: [pluginTs({ output: { path: 'types', barrel: false } })],
})
