const http = require('http');
let requests = 0;
http.createServer((req, res) => {
  if (req.url === '/stats') { res.setHeader('content-type','application/json'); res.end(JSON.stringify({requests})); return; }
  if (req.url !== '/v1/chat/completions' || req.method !== 'POST') { res.statusCode=501; res.end('unmodelled route'); return; }
  let raw=''; req.on('data',chunk=>raw+=chunk); req.on('end',()=>{
    requests++;
    const body=JSON.parse(raw);
    const content=JSON.stringify({ reply:'后台执行完成，页面重建后仍可读取。', title:'完成',summary:'任务恢复验证',risks:[],conventions:[] });
    setTimeout(()=>{
      if (body.stream) {
        res.setHeader('content-type','text/event-stream');
        res.write(`data: ${JSON.stringify({choices:[{index:0,delta:{role:'assistant',content}}]})}\n\n`);
        res.write(`data: ${JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]})}\n\n`);
        res.end('data: [DONE]\n\n');
      } else {
        res.setHeader('content-type','application/json');
        res.end(JSON.stringify({choices:[{message:{role:'assistant',content,tool_calls:null},finish_reason:'stop'}]}));
      }
    }, 5000);
  });
}).listen(process.env.PORT);
