(function() {
  const canvas = document.getElementById('hexCanvas');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  
  let mouseX = -999, mouseY = -999;
  const hexSize = 35;
  const hexH = hexSize * Math.sqrt(3);
  const influence = 250;

  function resize() {
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
  }
  
  resize();
  window.addEventListener('resize', resize);

  document.addEventListener('mousemove', (e) => {
    mouseX = e.clientX;
    mouseY = e.clientY + window.scrollY; // Adjust for scroll
  });

  function drawHex(cx, cy, size, alpha) {
    ctx.beginPath();
    for (let i = 0; i < 6; i++) {
      const angle = (Math.PI / 3) * i - Math.PI / 6;
      const x = cx + size * Math.cos(angle);
      const y = cy + size * Math.sin(angle);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.closePath();
    
    // Frontlane clay color for the glow: rgb(182, 78, 62)
    ctx.strokeStyle = `rgba(182, 78, 62, ${alpha})`;
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  function render() {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    
    // Subtle dark gradient background
    const grad = ctx.createRadialGradient(mouseX, mouseY - window.scrollY, 0, mouseX, mouseY - window.scrollY, influence * 2);
    grad.addColorStop(0, 'rgba(182, 78, 62, 0.08)');
    grad.addColorStop(1, 'rgba(13, 15, 18, 0)');
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    const cols = Math.ceil(canvas.width / (hexSize * 1.5)) + 2;
    const rows = Math.ceil(canvas.height / hexH) + 2;
    
    const scrollY = window.scrollY;

    for (let row = -1; row < rows; row++) {
      for (let col = -1; col < cols; col++) {
        const cx = col * hexSize * 1.5;
        const cy = row * hexH + (col % 2 === 1 ? hexH / 2 : 0);
        
        // Calculate distance from mouse to hex center
        // We must account for scroll because canvas is fixed but mouseY tracks document coordinates
        const screenCy = cy - scrollY;
        const dist = Math.hypot(mouseX - cx, mouseY - scrollY - screenCy);

        let alpha = 0.02; // Base faint grid
        if (dist < influence) {
          const t = 1 - (dist / influence);
          alpha += t * t * 0.4;
        }
        drawHex(cx, cy, hexSize, alpha);
      }
    }
    
    requestAnimationFrame(render);
  }

  render();
})();
