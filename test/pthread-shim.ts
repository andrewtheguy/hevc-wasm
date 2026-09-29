// Preloaded into each pthread worker the module starts. Emscripten's glue knows
// a worker by `WorkerGlobalScope` and a pthread by the worker's name, and Bun's
// workers have neither.
const scope = globalThis as { WorkerGlobalScope?: unknown; name?: string };
scope.WorkerGlobalScope ??= class WorkerGlobalScope {};
scope.name = "em-pthread";
