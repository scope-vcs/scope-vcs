// Try suffixes at the file probe, after aliases and extension substitution.
// This preserves extension priority and applies equally to directory indexes.
export function moduleSuffixPlugin(suffixes) {
  return {
    apply(resolver) {
      resolver.getHook("file").tapAsync(
        { name: "ScopeModuleSuffixes", stage: -10 },
        (request, context, callback) => {
          const extension = request.path?.match(/(?:\.d)?\.(?:[cm]?ts|tsx|[cm]?js|jsx|json)$/u)?.[0];
          if (!extension) return callback();
          let index = 0;
          const next = () => {
            // A null result stops the unsuffixed file probe when "" is absent.
            if (index === suffixes.length) return callback(null, null);
            const suffix = suffixes[index++];
            const path = request.path.slice(0, -extension.length) + suffix + extension;
            resolver.fileSystem.stat(path, (error, metadata) => {
              if (error || !metadata?.isFile()) return next();
              resolver.doResolve(resolver.ensureHook("existing-file"), {
                ...request,
                path,
                relativePath: request.relativePath
                  ? request.relativePath.slice(0, -extension.length) + suffix + extension
                  : request.relativePath,
              }, "TypeScript module suffix", context, callback);
            });
          };
          next();
        },
      );
    },
  };
}
