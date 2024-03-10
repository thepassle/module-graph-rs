import { createModuleGraph as create_module_graph, /*getCwd*/ } from './index.cjs';
import { builtinModules } from "module";


// getCwd((foo,bar) => {
//   console.log(foo, bar);
//   return false;
//   return 'string';
// });

export function createModuleGraph(entrypoints, options = {}) {
  const { 
    plugins = [
      // {
        // name: 'foo',
        // start: (entrypoint) => {
        //   console.log(111, entrypoint);
        // },
        // analyze: (source) => {
        //   console.log(222, source);
        // },
        // handleImport(importer, importee) {
        //   console.log(333, importer, importee);
        //   return 'f00';
        // }
      // }
    ], 
    basePath = process.cwd(), 
    exportConditions = ["node", "import"],
    ignoreExternal = false,
    ...resolveOptions 
  } = options;

  const processedEntrypoints = (typeof entrypoints === "string" ? [entrypoints] : entrypoints);
  const result = create_module_graph(
    processedEntrypoints, 
    basePath, 
    exportConditions, 
    builtinModules, 
    ignoreExternal, 
    plugins
  );

  /**
   * This logic is only here to achieve compatibility with the test suite
   * so I can more easily port over the tests, we may keep it here or decide 
   * to just go with objects/arrays
   */
  const graph = new Map();
  for (const [mod, deps] of Object.entries(result.graph)) {
    graph.set(mod, new Set(deps));
  }
  const modules = new Map();
  for (const [p, mod] of Object.entries(result.modules)) {
    modules.set(p, mod);
  }

  const moduleGraph = {
    basePath,
    entrypoints: result.entryPoints,
    graph, 
    modules,
    // @TODO placeholder for now
    get(mod) {
      return [modules.get(mod)];
    },
    getUniqueModules() {
      let result = [];
      for (const [, v] of modules) {
        result.push(v.path);
      }
      return result;
    }
  };

  for (const { end } of plugins) {
    end?.(moduleGraph);
  }

  return moduleGraph;
}

// const graph = createModuleGraph('./a.js');
// console.log(graph);


// console.assert(plus100(0) === 100, 'Simple test failed')

// plus100((p) => {
//   console.log(p);
//   return 'foo';
// });

// console.info('Simple test passed')
