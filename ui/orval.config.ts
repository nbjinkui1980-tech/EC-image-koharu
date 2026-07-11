import { defineConfig } from 'orval'

export default defineConfig({
  koharu: {
    input: './openapi.json',
    output: {
      target: './lib/api/generated.ts',
      schemas: './lib/api/schemas',
      client: 'fetch',
      mode: 'single',
      baseUrl: '/api/v1',
      mock: false,
      clean: ['!index.ts', '!fetch.ts'],
      override: {
        fetch: {
          includeHttpResponseReturnType: false,
        },
        mutator: {
          path: './lib/api/fetch.ts',
          name: 'fetchApi',
        },
        operations: {
          createPages: {
            formData: true,
          },
          addImageLayer: {
            formData: true,
          },
        },
      },
    },
  },
})
