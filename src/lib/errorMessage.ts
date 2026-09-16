/** Tauri rejects with serialized AppError objects, while browser code throws Error. */
export function errorMessage(error: unknown): string {
  if (
    error &&
    typeof error === "object" &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return typeof error === "string"
    ? error
    : "The operation failed. Please retry.";
}
