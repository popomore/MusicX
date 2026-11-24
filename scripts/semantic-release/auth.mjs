export function semanticEnvironment(env) {
  const prepared = { ...env };
  if (env.GITHUB_ACTIONS === 'true' && (env.GITHUB_TOKEN || env.GH_TOKEN)) {
    // The pinned engine treats GH_TOKEN as a PAT username. Installation tokens
    // need its GITHUB_TOKEN path, which adds the x-access-token: Basic prefix.
    prepared.GITHUB_TOKEN = env.GITHUB_TOKEN || env.GH_TOKEN;
    prepared.GITHUB_ACTION = env.GITHUB_ACTION || 'semantic-release';
    delete prepared.GH_TOKEN;
  }
  return prepared;
}
