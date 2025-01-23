class Icon {
    constructor(url) {
        this.url = url;
        this.svgContent = '';
        this.svgLoaded = false;
        this.cacheCanvas = null; // Offscreen canvas for caching
        this.cacheContext = null;
        this.cacheColor = null;
        this.load();
    }

    async load() {
        try {
            const response = await fetch(this.url);
            this.svgContent = await response.text();
            this.svgLoaded = true;
        } catch (error) {
            console.error('Error loading SVG:', error);
            this.svgLoaded = false;
        }
    }

    render(ctx, x, y, w, h, color) {
        if (!this.svgLoaded) return; // Do nothing if SVG is not loaded

        if (!this.cacheCanvas || this.cacheCanvas.width !== w || this.cacheCanvas.height !== h || this.cacheColor !== color) {
            // Create or update the offscreen canvas for caching
            this.cacheCanvas = document.createElement('canvas');
            this.cacheCanvas.width = w;
            this.cacheCanvas.height = h;
            this.cacheContext = this.cacheCanvas.getContext('2d');

            // Draw the SVG onto the offscreen canvas
            this.cacheContext.clearRect(0, 0, w, h); // Clear previous contents
            this.cacheContext.fillStyle = color;
            this.cacheColor = color
            this.cacheContext.drawSvg(this.svgContent, 0, 0, w, h);
        }

        // Draw the cached result onto the main canvas
        ctx.drawImage(this.cacheCanvas, x, y);
    }
}

export { Icon };
