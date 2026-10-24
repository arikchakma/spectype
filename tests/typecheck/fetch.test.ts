import createClient from "openapi-fetch";
import {
  errorNameValues,
  type ErrorBody,
  type ErrorName,
  type paths,
  type ProjectBody,
} from "./generated/fetch.primary";

const client = createClient<paths>({ baseUrl: "https://api.example.test" });

export async function pathParamsAndBody() {
  const body: ProjectBody = { name: "Launch", notes: null, settings: { labels: [] } };
  const { data, error, response } = await client.PATCH("/v1/projects/{project_id}", {
    params: { path: { project_id: "prj_1" }, header: { "Idempotency-Key": "k" } },
    body,
  });
  if (data) {
    const teams: { id: string; name: string }[] = data.teams;
    void teams;
  }
  if (error) {
    const typed: ErrorBody = error;
    const name: ErrorName = error.name;
    const status: number = error.status;
    void typed, name, status;
  }
  void response.status;
}

export async function queryParams() {
  const { data } = await client.GET("/v1/teams/{team_id}/members", {
    params: { path: { team_id: "team_1" }, query: { per_page: 25, after: "c_9" } },
  });
  void data;

  await client.GET("/v1/teams/{team_id}/members", {
    // @ts-expect-error per_page is an integer
    params: { path: { team_id: "team_1" }, query: { per_page: "25" } },
  });

  await client.GET("/v1/teams/{team_id}/members", {
    // @ts-expect-error unknown query parameter
    params: { path: { team_id: "team_1" }, query: { nope: true } },
  });
}

export async function requiredPieces() {
  // @ts-expect-error the path parameter is required
  await client.PATCH("/v1/projects/{project_id}", { params: { path: {} }, body: {} });

  // @ts-expect-error the request body is required
  await client.PATCH("/v1/projects/{project_id}", { params: { path: { project_id: "prj_1" } } });

  // @ts-expect-error unknown path
  await client.GET("/v1/does-not-exist");
}

export function enumValues(value: string): value is ErrorName {
  return (errorNameValues as readonly string[]).includes(value);
}
