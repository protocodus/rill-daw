const {chromium}=require('playwright');
const fs=require('fs');
(async()=>{
  const b=__dirname;
  const browser=await chromium.launch({executablePath:'/usr/bin/chromium',headless:true,args:['--no-sandbox']});
  const page=await browser.newPage({viewport:{width:1024,height:900}});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.setContent(fs.readFileSync(b+'/rill-before-after.html','utf8'));
  await page.addStyleTag({content:':root{--font-size-base:14px;--foreground:#202832;--muted-foreground:#586471;color-scheme:light}body{font:14px system-ui;margin:16px}.form-select{max-width:100%;font:inherit;padding:8px}.form-label{display:grid;gap:6px}.text-muted{color:#586471}h3{margin:0}.viz-controls{display:flex;flex-wrap:wrap}'});
  const sizes=[];
  for(const width of [320,736,1024]){
    await page.setViewportSize({width,height:1000});
    for(const id of ['P1','P2','P3','P4','P5','P6']){
      await page.selectOption('#rill-proposal-choice',id);
      const result=await page.evaluate(()=>({documentWidth:document.documentElement.scrollWidth,viewport:innerWidth,visible:document.querySelectorAll('[data-proposal]:not([hidden])').length}));
      if(result.documentWidth>width)throw new Error(`${id} overflows ${width}: ${JSON.stringify(result)}`);
      if(result.visible!==1)throw new Error('Invalid visible panel count');
      sizes.push({width,id,passed:true});
    }
  }
  await page.selectOption('#rill-proposal-choice','P3');
  const step=page.locator('.rill-steps button').first();const before=await step.getAttribute('aria-pressed');await step.click();if(await step.getAttribute('aria-pressed')===before)throw new Error('Grid toggle failed');
  await page.selectOption('#rill-proposal-choice','P5');await page.selectOption('[data-scale-choice]','minor');if(!await page.locator('[data-third]').evaluate(e=>e.classList.contains('rill-outside')))throw new Error('Scale feedback failed');
  await page.selectOption('#rill-proposal-choice','P6');await page.selectOption('[data-export-type]','MP3');if(await page.locator('[data-bit-depth]').isVisible())throw new Error('MP3 bit depth visible');await page.selectOption('[data-export-type]','FLAC');if(await page.locator('[data-bit-depth] select').inputValue()!=='24-bit PCM')throw new Error('FLAC depth failed');
  await page.selectOption('#rill-proposal-choice','P1');
  await page.locator('[data-back-arranger]').click();
  await page.locator('[data-after-edit]').dblclick();
  if(await page.locator('[data-after-arranger]').isVisible() || !await page.locator('[data-after-editor]').isVisible())throw new Error('Focused editor navigation failed');
  await page.screenshot({path:b+'/preview-wide.png',fullPage:true});
  await page.setViewportSize({width:320,height:1000});await page.selectOption('#rill-proposal-choice','P3');await page.screenshot({path:b+'/preview-narrow.png',fullPage:true});
  if(errors.length)throw new Error(errors.join('\n'));
  fs.writeFileSync(b+'/mockup-check.json',JSON.stringify({widthChecks:sizes,jsErrors:errors,interactionChecks:['proposal selection','drum note toggle','scale guide feedback','export format conditional fields','focused clip editor navigation'],rendering:'Headless Chromium with minimal host-style fallback; actual host UI styles may differ.'},null,2));
  console.log('18 layout checks and 5 primary interaction checks passed; zero JavaScript errors.');
  await browser.close();
})().catch(e=>{console.error(e);process.exit(1)});
