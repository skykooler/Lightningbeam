// Selection actions: selectAll, selectNone, select

import { context, pointerList } from '../state.js';
import { arraysAreEqual } from '../utils.js';

// Forward declarations for injected dependencies
let undoStack = null;
let redoStack = null;
let updateUI = null;
let updateMenu = null;
let actions = null; // Reference to full actions object for self-calls

export function initializeSelectionActions(deps) {
  undoStack = deps.undoStack;
  redoStack = deps.redoStack;
  updateUI = deps.updateUI;
  updateMenu = deps.updateMenu;
  actions = deps.actions;
}

export const selectionActions = {
  selectAll: {
    create: () => {
      redoStack.length = 0;
      let selection = [];
      let shapeselection = [];
      const currentTime = context.activeObject.currentTime || 0;
      const layer = context.activeObject.activeLayer;
      for (let child of layer.children) {
        let idx = child.idx;
        const existsValue = layer.animationData.interpolate(`object.${idx}.exists`, currentTime);
        if (existsValue > 0) {
          selection.push(child.idx);
        }
      }
      // Use getVisibleShapes instead of currentFrame.shapes
      if (layer) {
        for (let shape of layer.getVisibleShapes(currentTime)) {
          shapeselection.push(shape.idx);
        }
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
      };
      undoStack.push({ name: "selectAll", action: action });
      actions.selectAll.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      updateUI();
      updateMenu();
    },
  },
  selectNone: {
    create: () => {
      redoStack.length = 0;
      let selection = [];
      let shapeselection = [];
      for (let item of context.selection) {
        selection.push(item.idx);
      }
      for (let shape of context.shapeselection) {
        shapeselection.push(shape.idx);
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
      };
      undoStack.push({ name: "selectNone", action: action });
      actions.selectNone.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
  },
  select: {
    create: () => {
      redoStack.length = 0;
      if (
        arraysAreEqual(context.oldselection, context.selection) &&
        arraysAreEqual(context.oldshapeselection, context.shapeselection)
      )
        return;
      let oldselection = [];
      let oldshapeselection = [];
      for (let item of context.oldselection) {
        oldselection.push(item.idx);
      }
      for (let shape of context.oldshapeselection) {
        oldshapeselection.push(shape.idx);
      }
      let selection = [];
      let shapeselection = [];
      for (let item of context.selection) {
        selection.push(item.idx);
      }
      for (let shape of context.shapeselection) {
        shapeselection.push(shape.idx);
      }
      let action = {
        selection: selection,
        shapeselection: shapeselection,
        oldselection: oldselection,
        oldshapeselection: oldshapeselection,
      };
      undoStack.push({ name: "select", action: action });
      actions.select.execute(action);
      updateMenu();
    },
    execute: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.selection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.shapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
    rollback: (action) => {
      context.selection = [];
      context.shapeselection = [];
      for (let item of action.oldselection) {
        context.selection.push(pointerList[item]);
      }
      for (let shape of action.oldshapeselection) {
        context.shapeselection.push(pointerList[shape]);
      }
      updateUI();
      updateMenu();
    },
  },
};
