class Quadtree {
    constructor(boundary, capacity) {
      // Boundary is the bounding box of the area this quadtree node covers
      // Capacity is the maximum number of curves a node can hold before subdividing
      this.boundary = boundary; // {x: {min: <value>, max: <value>}, y: {min: <value>, max: <value>}}
      this.capacity = capacity;
      this.curveIndexes = [];
      this.curves = [];
      this.divided = false;
  
      this.nw = null; // Northwest quadrant
      this.ne = null; // Northeast quadrant
      this.sw = null; // Southwest quadrant
      this.se = null; // Southeast quadrant
    }
    
  
    // Check if a bounding box intersects with the boundary of this quadtree node
    intersects(bbox) {
      return !(bbox.x.max < this.boundary.x.min || bbox.x.min > this.boundary.x.max ||
               bbox.y.max < this.boundary.y.min || bbox.y.min > this.boundary.y.max);
    }
  
    // Subdivide this quadtree node into 4 quadrants
    subdivide() {
      const xMid = (this.boundary.x.min + this.boundary.x.max) / 2;
      const yMid = (this.boundary.y.min + this.boundary.y.max) / 2;
  
      const nwBoundary = { x: { min: this.boundary.x.min, max: xMid }, y: { min: this.boundary.y.min, max: yMid }};
      const neBoundary = { x: { min: xMid, max: this.boundary.x.max }, y: { min: this.boundary.y.min, max: yMid }};
      const swBoundary = { x: { min: this.boundary.x.min, max: xMid }, y: { min: yMid, max: this.boundary.y.max }};
      const seBoundary = { x: { min: xMid, max: this.boundary.x.max }, y: { min: yMid, max: this.boundary.y.max }};
  
      this.nw = new Quadtree(nwBoundary, this.capacity);
      this.ne = new Quadtree(neBoundary, this.capacity);
      this.sw = new Quadtree(swBoundary, this.capacity);
      this.se = new Quadtree(seBoundary, this.capacity);
  
      this.divided = true;
    }

    insert (curve, curveIdx) {
        const bbox = curve.bbox()
        if (!this.intersects(curve.bbox())) {
            let newNode = new Quadtree(this.boundary, this.capacity)
            newNode.curveIndexes = this.curveIndexes;
            newNode.curves = this.curves;
            newNode.divided = this.divided;

            newNode.nw = this.nw;
            newNode.ne = this.ne;
            newNode.sw = this.sw;
            newNode.se = this.se;

            this.curveIndexes = [];
            this.curves = [];
            this.subdivide()
            if (bbox.x.max < this.boundary.x.max) {
                if (bbox.y.max < this.boundary.y.max) {
                    this.boundary.x.min -= this.boundary.x.max - this.boundary.x.min
                    this.boundary.y.min -= this.boundary.y.max - this.boundary.y.min
                    this.nw = newNode
                } else {
                    this.boundary.x.min -= this.boundary.x.max - this.boundary.x.min
                    this.boundary.y.max += this.boundary.y.max - this.boundary.y.min
                    this.sw = newNode
                }
            } else {
                if (bbox.y.max < this.boundary.y.max) {
                    this.boundary.x.max += this.boundary.x.max - this.boundary.x.min
                    this.boundary.y.min -= this.boundary.y.max - this.boundary.y.min
                    this.ne = newNode
                } else {
                    this.boundary.x.max += this.boundary.x.max - this.boundary.x.min
                    this.boundary.y.max += this.boundary.y.max - this.boundary.y.min
                    this.se = newNode
                }
            }
            return this.insert(curve, curveIdx)
        } else {
            return this._insert(curve, curveIdx)
        }
    }
  
    // Insert a curve into the quadtree, subdividing if necessary
    _insert(curve, curveIdx) {
      // If the curve's bounding box doesn't intersect this node's boundary, do nothing
      if (!this.intersects(curve.bbox())) {
        return false;
      }
  
      // If the node has space, insert the curve here
      if (this.curves.length < this.capacity) {
        this.curves.push(curve);
        this.curveIndexes.push(curveIdx)
        return true;
      }
  
      // Otherwise, subdivide and insert the curve into the appropriate quadrant
      if (!this.divided) {
        this.subdivide();
      }
  
      return (
        this.nw._insert(curve, curveIdx) || 
        this.ne._insert(curve, curveIdx) || 
        this.sw._insert(curve, curveIdx) || 
        this.se._insert(curve, curveIdx)
      );
    }
  
    // Query all curves that intersect with a given bounding box
    query(range, found = []) {
      // If the range doesn't intersect with this node's boundary, return
      if (!this.intersects(range)) {
        return found;
      }
  
    // Check the curves in this node
    for (let i = 0; i < this.curves.length; i++) {
        if (this.bboxIntersect(this.curves[i].bbox(), range)) {
          found.push(this.curveIndexes[i]);  // Return the curve index instead of the curve
        }
      }
  
      // If the node is subdivided, check the child quadrants
      if (this.divided) {
        this.nw.query(range, found);
        this.ne.query(range, found);
        this.sw.query(range, found);
        this.se.query(range, found);
      }
  
      return found;
    }
  
    // Helper method to check if two bounding boxes intersect
    bboxIntersect(bbox1, bbox2) {
      return !(bbox1.x.max < bbox2.x.min || bbox1.x.min > bbox2.x.max ||
               bbox1.y.max < bbox2.y.min || bbox1.y.min > bbox2.y.max);
    }

    clear() {
        this.curveIndexes = [];
        this.curves = [];
        this.divided = false;
    
        this.nw = null; // Northwest quadrant
        this.ne = null; // Northeast quadrant
        this.sw = null; // Southwest quadrant
        this.se = null; // Southeast quadrant
    }
    draw(ctx) {
        // Debug visualization
        ctx.save()
        ctx.strokeStyle = "red"
        ctx.lineWidth = 1
        ctx.beginPath()
        ctx.rect(
            this.boundary.x.min,
            this.boundary.y.min,
            this.boundary.x.max-this.boundary.x.min,
            this.boundary.y.max-this.boundary.y.min
        )
        ctx.stroke()
        if (this.divided) {
            this.nw.draw(ctx)
            this.ne.draw(ctx)
            this.sw.draw(ctx)
            this.se.draw(ctx)
        }
        ctx.restore()
    }
  }

  export { Quadtree };