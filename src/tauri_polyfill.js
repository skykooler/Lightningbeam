if (!window.__TAURI__) {
  // We are in a browser environment
  window.__TAURI__ = {
    core: {
      invoke: () => {}
    },
    fs: {
      writeFile: () => {},
      readFile: () => {},
      writeTextFile: () => {},
      readTextFile: () => {}
    },
    dialog: {
      open: () => {},
      save: () => {},
      message: () => {},
      confirm: () => {},
    },
    path: {
      documentDir: () => {},
      join: () => {},
      basename: () => {},
      appLocalDataDir: () => {}
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
      getCurrentWindow: () => {}
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