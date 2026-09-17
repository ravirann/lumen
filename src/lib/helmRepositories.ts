import { invoke } from "@tauri-apps/api/core";

export interface HelmRepository {
  name: string;
  url: string;
}

/** Uses the local Helm CLI configuration shared by all clusters on this machine. */
export const helmRepositories = {
  list: () => invoke<HelmRepository[]>("helm_repository_list"),
  add: (name: string, url: string) =>
    invoke<void>("helm_repository_add", { name, url }),
  remove: (name: string) => invoke<void>("helm_repository_remove", { name }),
  update: () => invoke<void>("helm_repository_update"),
};
