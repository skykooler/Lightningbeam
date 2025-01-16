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

  function promptForFilename(filters, defaultFilename = '') {
    function createLabel(text, forId) {
      const label = document.createElement('label');
      label.setAttribute('for', forId);
      label.textContent = text;
      return label;
    }
    return new Promise((resolve, reject) => {
      // Create and style modal dynamically
      const modal = document.createElement('div');
      const modalContent = document.createElement('div');
      const filenameInput = document.createElement('input');
      const fileFilter = document.createElement('select');
      const submitBtn = document.createElement('button');
      const cancelBtn = document.createElement('button');
      
      // Append elements
      modal.appendChild(modalContent);
      modalContent.appendChild(createLabel('Enter filename:', 'filenameInput'));
      modalContent.appendChild(filenameInput);
      modalContent.appendChild(createLabel('Select file type:', 'fileFilter'));
      modalContent.appendChild(fileFilter);
      modalContent.appendChild(submitBtn);
      modalContent.appendChild(cancelBtn);

      document.body.appendChild(modal);

      // Style modal elements
      modal.id = "saveOverlay";
      modal.style.display = 'block';
      modalContent.id = "saveDialog";
      modalContent.style.display = 'block';
      [filenameInput, fileFilter].forEach(el => Object.assign(el.style, {
        width: '100%', padding: '10px', marginBottom: '10px'
      }));
      
      // Populate filter dropdown and set default filename
      filters.forEach(filter => fileFilter.add(new Option(filter.name, filter.extensions[0])));
      filenameInput.value = defaultFilename
      const extension = defaultFilename.split('.').pop();
      filenameInput.focus()
      filenameInput.setSelectionRange(0, defaultFilename.length - extension.length - 1);  // Select only the base filename
        
      // Update extension based on selected filter
      fileFilter.addEventListener('change', () => updateFilename(true));
      filenameInput.addEventListener('input', () => updateFilename(false));

      function updateFilename(reselect) {
        const base = filenameInput.value.split('.')[0];
        filenameInput.value = `${base}.${fileFilter.value}`;
        if (reselect) {
          filenameInput.focus()
          filenameInput.setSelectionRange(0, base.length);  // Select only the base filename
        }
      }

      // Handle buttons
      submitBtn.textContent = 'Submit';
      cancelBtn.textContent = 'Cancel';
      submitBtn.onclick = () => {
        const chosenFilename = filenameInput.value;
        if (!chosenFilename) reject(new Error('Filename missing.'));
        resolve(chosenFilename);
        modal.remove();
      };
      cancelBtn.onclick = () => {
        reject(new Error('User canceled.'));
        modal.remove();
      };
      
      // Close modal if clicked outside
      window.addEventListener('click', (e) => {
        if (e.target === modal) {
          reject(new Error('User clicked outside.'));
          modal.remove();
        }
      });
    });
  }

  window.__TAURI__ = {
    core: {
      invoke: () => {}
    },
    fs: {
      writeFile: (path, contents) => {
        // Create a Blob from the contents
        const blob = new Blob([contents]);
        const link = document.createElement('a');
        const url = URL.createObjectURL(blob);
    
        link.href = url;
        link.download = path;  // Use the file name from the path
    
        document.body.appendChild(link);
        link.click();
    
        // Clean up by removing the link and revoking the object URL
        document.body.removeChild(link);
        URL.revokeObjectURL(url);
      },
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
      save: async (params) => {
        return await promptForFilename(params.filters, params.defaultPath)
      },
      message: () => {},
      confirm: () => {},
    },
    path: {
      documentDir: () => {},
      join: (...segments) => {
        return segments.filter(segment => (segment && segment.length > 0))  // Remove empty strings
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
            if (item instanceof Menu) {
              menuItem = new MenuItem({
                label: item.text,
                submenu: item
              })
            } else {
              menuItem = new MenuItem({
                label: item.text,
                enabled: item.enabled,
                click: item.action,
                accelerator: item.accelerator
              })
            }
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