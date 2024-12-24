if (!window.__TAURI__) {
  // We are in a browser environment
  const fileContentsDict = {}

  // Open IndexedDB database for storing files
  const openDb = () => {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open('fileStorage', 1);

      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains('files')) {
          db.createObjectStore('files', { keyPath: 'path' });
        }
      };

      request.onsuccess = () => {
        resolve(request.result);
      };

      request.onerror = () => {
        reject('Error opening IndexedDB');
      };
    });
  };

  // Retrieve a file from IndexedDB by path
  const getFileFromIndexedDB = async (path) => {
    const db = await openDb();
    const transaction = db.transaction('files', 'readonly');
    const store = transaction.objectStore('files');
    
    return new Promise((resolve, reject) => {
      const request = store.get(path);  // Get file by path (key)
      
      request.onsuccess = () => {
        if (request.result) {
          resolve(request.result);
        } else {
          reject('File not found');
        }
      };
      
      request.onerror = () => {
        reject('Error retrieving file from IndexedDB');
      };
    });
  };

  window.__TAURI__ = {
    core: {
      invoke: () => {}
    },
    fs: {
      writeFile: () => {},
      readFile: () => {},
      writeTextFile: async (path, contents) => {
        // Create a Blob from the contents
        const blob = new Blob([contents], { type: 'application/json' });
        const link = document.createElement('a');
        const url = URL.createObjectURL(blob);

        // Store the file in IndexedDB
        const db = await openDb();
        const transaction = db.transaction('files', 'readwrite');
        const store = transaction.objectStore('files');
        
        const fileData = {
          path: path,
          content: contents,
          blob: blob,
          date: new Date().toISOString()  // Optional: store when the file was saved
        };
        
        store.put(fileData);  // Store the file data (with path as key)

        // Handle IndexedDB errors
        transaction.onerror = (e) => {
          console.error('Error storing file in IndexedDB:', e.target.error);
        };
    
        link.href = url;
        link.download = path;  // Use the file name from the path
    
        document.body.appendChild(link);
        link.click();
    
        // Clean up by removing the link and revoking the object URL
        document.body.removeChild(link);
        URL.revokeObjectURL(url);
      },
      readTextFile: async (path) => {
        return new Promise(async (resolve, reject) => {
          // Check if the file exists in the dictionary
          const contents = fileContentsDict[path];
          if (contents) {
            resolve(contents); // Return the file contents
          } else {
            try {
              // If not found in the dictionary, try fetching it from IndexedDB
              const fileData = await getFileFromIndexedDB(path);
              
              if (fileData) {
                // Store the contents in the dictionary for future use
                fileContentsDict[path] = fileData.content;
                
                // Resolve with the file contents
                resolve(fileData.content);
              } else {
                // Reject if the file is not found in IndexedDB
                reject(new Error('File not found in IndexedDB.'));
              }
            } catch (error) {
              // Reject if there is an error retrieving from IndexedDB
              reject(new Error('Error retrieving file from IndexedDB: ' + error.message));
            }
          }
        });
      }
    },
    dialog: {
      open: async (dialogOptions = {}) => {
        return new Promise((resolve, reject) => {
          // Create a file input element for the user to select a file
          const input = document.createElement('input');
          input.type = 'file';
          let accept = '';
          let filters = dialogOptions.filters

          if (filters && filters.length > 0) {
            // Convert each filter to a valid file extension string for the accept attribute
            const acceptArray = filters.map(filter => {
              // Each filter's extensions array can be converted to `*.ext` format
              return filter.extensions.map(ext => `.${ext}`).join(',');
            });

            // Join all filters into one string, separated by commas (for multiple types)
            accept = acceptArray.join(',');
          }

          
          input.accept = accept;

          // Set up an event handler for when the file is selected
          input.onchange = async () => {
            const file = input.files[0];
            if (file) {
              const reader = new FileReader();
              
              // When the file is read, store its contents in the global dictionary
              reader.onload = () => {
                // Store the file content in the dictionary with the file name as the key
                fileContentsDict[file.name] = reader.result;
                resolve(file.name); // Resolve with the file path (name)
              };

              reader.onerror = () => {
                reject(new Error('Failed to read file.'));
              };

              // Start reading the file as text
              reader.readAsText(file);
            } else {
              reject(new Error('No file selected.'));
            }
          };

          // Trigger the file input dialog by simulating a click
          input.click();
        });
      },
      save: async () => {
        return prompt("Filename", "untitled.beam")
      },
      message: () => {},
      confirm: () => {},
    },
    path: {
      documentDir: () => {},
      join: (...segments) => {
        return segments.filter(segment => segment.length > 0)  // Remove empty strings
        .join('/')
      },
      basename: (path) => {
        // Ensure the path is a string and remove any leading or trailing whitespace
        path = path.trim();
      
        // Remove the directory part, leaving only the file name (if there is one)
        const lastSlashIndex = path.lastIndexOf('/'); // Look for the last '/' in the path
        if (lastSlashIndex === -1) {
          return path;
        } else {
          return path.slice(lastSlashIndex + 1);
        }
      },
      appLocalDataDir: () => {},
      dirname: (path) => {
        path = path.trim();
      
        const lastSlashIndex = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
      
        if (lastSlashIndex === -1) {
          return '';
        } else {
          return path.slice(0, lastSlashIndex);
        }
      }
    },
    menu: {
      Menu: {
        new: (params) => {
          let items = params.items
          let menubar = new Menu({type: "menubar"})
          for (let i in items) {
            let item = items[i]
            menubar.append(new MenuItem({label: item.text, submenu: item}))
          }
          menubar.setAsWindowMenu = () => {
            Menu.setApplicationMenu(menubar)
          }
          menubar.setAsAppMenu = menubar.setAsWindowMenu
          return menubar
        }
      },
      MenuItem: MenuItem,
      PredefinedMenuItem: () => {},
      Submenu: {
        new: (params) => {
          const items = params.items
          menu = new Menu()
          for (let i in items) {
            let item = items[i]
            menuItem = new MenuItem({
              label: item.text,
              enabled: item.enabled,
              click: item.action,
              accelerator: item.accelerator
            })
            menu.append(menuItem)
          }
          menu.text = params.text
          return menu
        }
      }
    },
    window: {
      getCurrentWindow: () => {
        return {
          setTitle: (title) => {
            document.title = title
          }
        }
      }
    },
    app: {
      getVersion: () => {}
    },
    log: {
      warn: () => {},
      debug: () => {},
      trace: () => {},
      info: () => {},
      error: () => {},
    }
  }
}