class Icon {
    constructor(url) {
        this.url = url;
        this.svgContent = '';
        this.svgLoaded = false;
        this.load()
    }

    async load() {
        return fetch(this.url)
            .then(response => response.text())
            .then(svgContent => {
                this.svgContent = svgContent;
                this.svgLoaded = true;
            })
            .catch(error => {
                console.error('Error loading SVG:', error);
                this.svgLoaded = false;
            });
    }

    render(ctx, x, y, w, h, color) {
        if (this.svgLoaded) {
            ctx.fillStyle = color
            ctx.drawSvg(this.svgContent, x, y, w, h)
        }
    }
}

export { Icon };