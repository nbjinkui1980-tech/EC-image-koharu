import { setupServer } from 'msw/node'

// Tests register only the endpoints they exercise. Unhandled requests fail,
// so adding a new backend dependency requires an explicit deterministic mock.
export const server = setupServer()
