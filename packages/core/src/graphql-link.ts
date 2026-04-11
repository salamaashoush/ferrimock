/**
 * URL-scoped GraphQL handlers (MSW's `graphql.link(url)`).
 *
 * Returns `{ query, mutation, operation }` methods that only match
 * GraphQL requests sent to the specified endpoint URL.
 */

import { graphql } from "@mockpit/node";
import type { JsHandler } from "@mockpit/node";

type HandlerFn = Parameters<typeof graphql.query>[1];

interface GraphQLLinkHandlers {
  query(operationName: string, handler: HandlerFn): JsHandler;
  mutation(operationName: string, handler: HandlerFn): JsHandler;
  operation(handler: HandlerFn): JsHandler;
}

/**
 * Create URL-scoped GraphQL handlers.
 *
 * @param url - The GraphQL endpoint URL to match (string or RegExp).
 * @returns Object with `query`, `mutation`, `operation` methods.
 *
 * @example
 * ```ts
 * const github = graphqlLink('https://api.github.com/graphql')
 * server.useHandlers([
 *   github.query('GetUser', () => MockResponse.json({ data: { user: {} } })),
 * ])
 * ```
 */
export function graphqlLink(url: string | RegExp): GraphQLLinkHandlers {
  const urlMatcher =
    typeof url === "string"
      ? (reqUrl: string) => reqUrl === url || reqUrl.endsWith(url)
      : (reqUrl: string) => url.test(reqUrl);

  function wrapHandler(handler: HandlerFn): HandlerFn {
    return async (req: any) => {
      // Check if request URL matches the scoped endpoint
      if (!urlMatcher(req.uri)) {
        return null; // passthrough — URL doesn't match this endpoint
      }
      return handler(req);
    };
  }

  return {
    query(operationName: string, handler: HandlerFn): JsHandler {
      return graphql.query(operationName, wrapHandler(handler));
    },
    mutation(operationName: string, handler: HandlerFn): JsHandler {
      return graphql.mutation(operationName, wrapHandler(handler));
    },
    operation(handler: HandlerFn): JsHandler {
      return graphql.operation(wrapHandler(handler));
    },
  };
}
