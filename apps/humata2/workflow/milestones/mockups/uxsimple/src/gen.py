import os, html
OUT = os.path.dirname(os.path.abspath(__file__))
CSS = """
*{box-sizing:border-box;margin:0;padding:0}
body{width:1600px;height:950px;font-family:-apple-system,"SF Pro Text",Helvetica,sans-serif;font-size:14px;color:#1d1d1f;background:#fff;overflow:hidden;position:relative;display:flex}
.mono{font-family:"SF Mono",Menlo,monospace;font-size:13px}
.sp{flex:1}.dim{color:#8e8e93}.g{color:#28a745}.r{color:#e0393e}.o{color:#d98200}.b{color:#0a6cf0}
.dot{width:9px;height:9px;border-radius:50%;display:inline-block;flex:none}
.sb{width:280px;background:#f5f5f7;border-right:1px solid #e3e3e6;padding:22px 12px;flex:none;display:flex;flex-direction:column;gap:2px}
.lights{display:flex;gap:8px;margin:0 0 22px 8px}.lights i{width:13px;height:13px;border-radius:50%;display:block}
.search{background:#ebebef;border-radius:7px;padding:6px 10px;color:#8e8e93;margin-bottom:12px;display:flex}
.nav{padding:6px 10px;display:flex;gap:10px;align-items:center;border-radius:7px;white-space:nowrap}
.nav.on{background:#0a6cf0;color:#fff;font-weight:600}
.sec{color:#8e8e93;font-size:12px;font-weight:600;padding:16px 10px 4px}
.main{flex:1;display:flex;flex-direction:column;min-width:0}
.hdr{height:65px;border-bottom:1px solid #e3e3e6;display:flex;align-items:center;padding:0 18px;gap:12px;flex:none}
.crumb{font-size:13px;color:#0a6cf0}.crumb.q{color:#6e6e73}
.h{font-weight:600;font-size:16px}
.tabs{height:40px;border-bottom:1px solid #e3e3e6;display:flex;align-items:stretch;background:#fbfbfd;flex:none}
.tab{display:flex;align-items:center;gap:8px;padding:0 14px;border-right:1px solid #e3e3e6;color:#424245;white-space:nowrap}
.tab.on{background:#fff;color:#1d1d1f;box-shadow:inset 0 -2px 0 #0a6cf0}
.x{color:#8e8e93;font-size:12px}
.out{padding:14px 18px;line-height:1.62;white-space:pre;flex:1;overflow:hidden}
.out .u{font-weight:600}
.prompt{margin:0 18px 8px;border:1px solid #c7c7cc;border-radius:5px;padding:8px 10px}
.hint{margin:0 18px 14px;color:#8e8e93}
.panel{border-left:1px solid #e3e3e6;display:flex;flex-direction:column;flex:none;background:#fff}
.ph{height:44px;border-bottom:1px solid #e3e3e6;display:flex;align-items:center;padding:0 14px;gap:10px;flex:none;white-space:nowrap}
.rail{width:76px;border-left:1px solid #e3e3e6;background:#fbfbfd;flex:none;padding-top:70px;display:flex;flex-direction:column;gap:22px;align-items:center;font-size:11px;color:#6e6e73}
.rail div{text-align:center}.rail i{display:block;width:20px;height:18px;border:1.5px solid #8e8e93;border-radius:4px;margin:0 auto 4px}
.rail .on{color:#0a6cf0}.rail .on i{border-color:#0a6cf0}
.chip{border:1px solid #d9d9de;border-radius:7px;padding:3px 9px;background:#f5f5f7;white-space:nowrap}
.btn{border:1px solid #d9d9de;border-radius:7px;padding:5px 12px;background:#f5f5f7;white-space:nowrap}
.btn.primary{background:#0a6cf0;border-color:#0a6cf0;color:#fff;font-weight:500}
.btn.ok{background:#e9f7ed;border-color:#b7e0c2;color:#1c7c36}
.link{color:#0a6cf0;white-space:nowrap}
.seg{display:flex;border:1px solid #d9d9de;border-radius:7px;overflow:hidden;background:#f5f5f7}
.seg span{padding:3px 10px;white-space:nowrap}.seg .on{background:#fff;box-shadow:0 0 0 1px #d9d9de;font-weight:500}
.list{background:#f5f5f7;padding:10px 8px;overflow:hidden}
.cap{color:#8e8e93;font-size:12px;font-weight:600;padding:6px 10px 4px;display:flex}
.ly{display:flex;align-items:flex-start;padding:6px 10px;border-radius:7px;gap:9px}
.ly .n{width:20px;height:20px;border-radius:50%;border:1px solid #c7c7cc;text-align:center;font-size:12px;line-height:18px;flex:none;color:#424245;background:#fff}
.ly.sel{background:#e4e4ea}.ly.sel .n{background:#0a6cf0;border-color:#0a6cf0;color:#fff}
.ly .pp{color:#6e6e73;font-size:12.5px;margin-top:2px;white-space:normal}
.ly .st{font-size:12.5px;white-space:nowrap;margin-top:1px}
.sep{height:1px;background:#e3e3e6;margin:8px 4px}
.f{display:flex;align-items:center;padding:5px 10px;border-radius:7px;gap:7px;white-space:nowrap}
.f.sel{background:#e4e4ea}
.f.file{padding-left:30px}
.ck{width:15px;height:15px;border-radius:50%;border:1.5px solid #b0b0b6;flex:none;font-size:10px;line-height:12px;text-align:center;color:#fff}
.ck.on{background:#28a745;border-color:#28a745}
.ck.ch{border-color:#d98200;background:#fff3dc}
.code{flex:1;display:flex;flex-direction:column;min-width:0;overflow:hidden}
.fb{height:38px;border-bottom:1px solid #e3e3e6;display:flex;align-items:center;padding:0 14px;gap:8px;color:#6e6e73;flex:none;white-space:nowrap}
.hunk{color:#6e6e73;padding:8px 12px 2px}
.ln{display:flex;padding:1px 12px;white-space:pre}.ln span:first-child{width:52px;color:#8e8e93;flex:none}
.del{background:#fde8e8}.add{background:#e6f6ea}
.card{border:1px solid #d9d9de;border-radius:8px;margin:10px 12px;padding:10px 12px;background:#fff;line-height:1.45}
.card.draft{border-color:#0a6cf0;box-shadow:0 0 0 1px #0a6cf0}
.card.done{background:#f7f7f9;color:#6e6e73}
.card .top{display:flex;gap:8px;align-items:center;margin-bottom:5px;white-space:nowrap}
.card .acts{display:flex;gap:16px;margin-top:8px}
.badge{font-size:12px;border-radius:5px;padding:1px 6px;border:1px solid #c9d8f5;color:#0a6cf0;background:#eef4ff}
.badge.gr{border-color:#b7e0c2;color:#1c7c36;background:#e9f7ed}
.badge.or{border-color:#f0d49a;color:#a86400;background:#fff6e3}
.reply{border-top:1px solid #e3e3e6;margin-top:8px;padding-top:8px}
.foot{height:52px;border-top:1px solid #e3e3e6;display:flex;align-items:center;padding:0 14px;gap:16px;flex:none;color:#424245;white-space:nowrap;background:#fff}
.strip{background:#eef4ff;border-bottom:1px solid #d6e4fb;padding:7px 14px;display:flex;gap:10px;align-items:center;white-space:nowrap;font-size:13px}
.menu{position:absolute;background:#fff;border:1px solid #d9d9de;border-radius:10px;box-shadow:0 10px 30px rgba(0,0,0,.16);padding:6px;z-index:5}
.mi{padding:6px 10px;border-radius:6px;display:flex;gap:9px;align-items:center;white-space:nowrap}.mi.on{background:#e8e8ed}
.mcap{color:#6e6e73;font-size:12.5px;padding:6px 10px 8px;line-height:1.4}
.col{display:flex;flex-direction:column;flex:1;min-width:0}
.row{display:flex;flex:1;min-height:0}
"""
def dot(c): return f'<span class="dot" style="background:{c}"></span>'
GREEN,ORANGE,GREY='#28c840','#f5a623','#b0b0b6'
LIGHTS='<div class="lights"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i></div>'

def sidebar(state=GREEN, badge=''):
    return f'''<div class="sb">{LIGHTS}
<div class="search">Search<span class="sp"></span>⌘K</div>
<div class="nav">Home</div><div class="nav">Inbox<span class="sp"></span><span class="dim">2</span></div><div class="nav">Board</div><div class="nav">Desk</div>
<div class="sec">Channels</div>
<div class="nav"><span class="dim">⌄</span><b>payments-rewrite</b></div>
<div class="nav on" style="margin-left:6px">{dot(state)}Ledger entries{badge}</div>
<div class="nav" style="margin-left:6px">{dot(GREEN)}Refund retries</div>
<div class="nav" style="margin-left:6px">{dot(GREY)}API rate limiter</div>
<div class="nav"><span class="dim">›</span>infra</div>
<div class="nav"><span class="dim">›</span>pr-reviews</div>
<div class="sec">Hosts</div>
<div class="nav">{dot(GREEN)}This Mac</div><div class="nav">{dot(GREEN)}devbox-1</div>
<div class="sp"></div><div class="nav">Settings</div></div>'''

def rail(on=True, n=''):
    d=f'<div class="{"on" if on else ""}"><i></i>Diff{n}</div>'
    return f'<div class="rail"><div><i></i>Inbox</div>{d}<div><i></i>Files</div><div><i></i>Board</div><div><i></i>Journal</div><div><i></i>Info</div></div>'

def session(tabs, out, state=GREEN, prompt='', right=''):
    th=''.join(f'<div class="tab {"on" if on else ""}">{dot(c) if c else "<span class=dim>&gt;_</span>"}{name}<span class="x">✕</span></div>' for name,c,on in tabs)
    return f'''<div class="main"><div class="hdr"><div><div class="crumb"># payments-rewrite ›</div><div style="display:flex;gap:8px;align-items:center;margin-top:3px">{dot(state)}<span class="h">Ledger entries</span></div></div><span class="sp"></span><span class="dim" style="white-space:nowrap">Claude Code · devbox-1</span><span class="dim">⑂ &nbsp;⤢ &nbsp;⋯</span></div>
<div class="tabs">{th}<div class="tab">+</div><span class="sp"></span>{right}</div>
<div class="out mono">{out}</div>
<div class="prompt mono">&gt; {prompt}<span style="background:#1d1d1f">&nbsp;</span></div><div class="hint mono">▸▸ accept edits on (shift+tab to cycle)</div></div>'''

def ly(n,title,st='',pp='',sel=False,prs=''):
    num = f'<span class="n">{n}</span>' if n else '<span class="n" style="border-style:dashed">≡</span>'
    p = f'<div class="pp">{pp}</div>' if pp else ''
    return f'<div class="ly {"sel" if sel else ""}">{num}<div style="min-width:0"><b style="font-weight:{600 if n else 500}">{title}</b> <span class="dim" style="font-size:12.5px">{prs}</span>{p}</div><span class="sp"></span><span class="st">{st}</span></div>'
OK='<span class="g">✓ Looks good</span>'
def repo(name, base='', stats=''):
    b = f'<span class="dim">{base}</span>' if base else ''
    return f'<div class="f"><span class="dim">⌄</span><b>{name}</b>{b}<span class="sp"></span>{stats}</div>'
def fl(name, d='', stats='', ck=0, sel=False, extra=''):
    c = {0:'<span class="ck"></span>',1:'<span class="ck on">✓</span>',2:'<span class="ck ch"></span>',None:''}[ck]
    return f'<div class="f file {"sel" if sel else ""}">{c}{name}<span class="dim">{d}</span><span class="sp"></span>{extra}{stats}</div>'
def pm(a,r): return f'<span class="g">+{a}</span> <span class="r">−{r}</span>'
def lines(rows, hunk=''):
    h = f'<div class="hunk">{html.escape(hunk)}</div>' if hunk else ''
    o=''
    for k,n,t in rows:
        cls={'+':'add','-':'del',' ':''}[k]
        sym={'+':'+','-':'−',' ':' '}[k]
        o+=f'<div class="ln {cls}"><span>{n:>3} {sym}</span><span>{html.escape(t)}</span></div>'
    return f'<div class="mono">{h}{o}</div>'
def card(who, body, tag='', acts='', cls='', reply=''):
    a = f'<div class="acts">{acts}</div>' if acts else ''
    rp = f'<div class="reply">{reply}</div>' if reply else ''
    return f'<div class="card {cls}"><div class="top"><b>{who}</b>{tag}</div>{body}{rp}{a}</div>'
def page(name, body):
    open(os.path.join(OUT,name+'.html'),'w').write(f'<!doctype html><meta charset="utf-8"><style>{CSS}</style><body>{body}</body>')

def full(title_extra, btns, left, code, foot, overlay=''):
    return f'''<div class="col"><div class="hdr">{LIGHTS}<div><div class="crumb q"># payments-rewrite › Ledger entries</div><div style="display:flex;gap:10px;align-items:center;margin-top:3px"><span class="h">Diff</span>{title_extra}</div></div><span class="sp"></span>{btns}</div>
<div class="row"><div class="list" style="width:420px;border-right:1px solid #e3e3e6;flex:none">{left}</div><div class="code">{code}</div></div>
<div class="foot">{foot}</div></div>{overlay}'''
def panel(w, head, body, foot, strip=''):
    return f'<div class="panel" style="width:{w}px"><div class="ph"><span class="h">Diff</span>{head}<span class="sp"></span><span class="dim">⤢ &nbsp; ✕</span></div>{strip}<div style="flex:1;overflow:hidden;display:flex;flex-direction:column">{body}</div><div class="foot">{foot}</div></div>'

T2=[('claude',GREEN,True),('zsh',None,False)]
FS_BTNS='<span class="btn">New agent tab ⌄</span><span class="btn">Exit full screen</span>'

# ---------- 1-1
out='''<span class="dim">~/src (ledger-entries) $ claude</span>
<span class="u">&gt; Move ledger reads to the new entries table. It touches api, web, ledger,
  sdk and infra. Make it a stack of small PRs I can review in order.</span>

● Read(ledger/docs/model.md)
  <span class="dim">⎿ Read 212 lines</span>
● Updated plan
  <span class="g">✓ Layer 1 · schema: accounts and entries tables (ledger, infra)</span>
  <span class="o">◐ Layer 2 · dual write: write both tables (api, ledger, sdk)</span>
  <span class="dim">  Layer 3 · reads: move reads to entries (api, web, sdk)</span>
● Write(ledger/migrations/0042_accounts.sql)
  <span class="dim">⎿ Wrote 41 lines</span>
● Write(infra/db/ledger.tf)
  <span class="dim">⎿ Wrote 18 lines</span>
● Update(api/src/client.rs)
  <span class="dim">⎿ Updated with 62 additions and 21 removals</span>
● Bash(bazel test //api/... //ledger/...)
  <span class="g">⎿ 31 tests pass</span>

<span class="o">✱ Writing the sdk types… (3m 02s · esc to interrupt)</span>'''
page('1-1-start-working', sidebar()+session(T2,out,right='<span class="tab dim" style="border:0">5 repos · devbox-1</span>')+rail(False,' · 9'))

# ---------- 1-2 stack made, plain diff panel
out='''● Bash(stack submit --all)
  <span class="dim">⎿ 3 layers · 8 pull requests</span>
  <span class="dim">  1 schema      acme/ledger#91  acme/infra#40</span>
  <span class="dim">  2 dual write  acme/api#482  acme/ledger#92  acme/sdk#17</span>
  <span class="dim">  3 reads       acme/api#483  acme/web#210  acme/sdk#18</span>

The stack is up: 3 layers across 5 repos, 23 files.
Each layer builds and passes its tests on its own.
Say "show me the stack" and I'll lay it out for
review in the Diff panel.'''
stack_plain = ly('', 'Whole stack', sel=True, prs='5 repos · 23 files')+ly(1,'schema',prs='2 PRs')+ly(2,'dual write',prs='3 PRs')+ly(3,'reads',prs='3 PRs')
body=f'''<div class="list" style="flex:1"><div class="cap">STACK · 3 LAYERS</div>{stack_plain}<div class="sep"></div>
{repo('api','against main',pm(205,45))}{fl('client.rs',' src/',pm(62,21))}{fl('entry.rs',' src/',pm(81,3))}{fl('reads.rs',' src/ledger/',pm(62,21))}
{repo('infra','against main',pm(18,0))}{fl('ledger.tf',' db/',pm(18,0))}
{repo('ledger','against release/2.4',pm(142,18))}{fl('0042_accounts.sql',' migrations/',pm(41,0))}{fl('0043_entries.sql',' migrations/',pm(38,0))}{fl('posting.rs',' src/',pm(63,18))}
{repo('sdk','against main',pm(96,12))}{fl('entry.ts',' src/',pm(44,8))}{fl('client.ts',' src/',pm(52,4))}
{repo('web','against main',pm(71,30))}{fl('LedgerTable.tsx',' src/ledger/',pm(71,30))}
<div class="f file dim">11 more files…</div></div>'''
page('1-2-stack-made', sidebar()+session(T2,out,prompt='show me the stack')+panel(500,'<span class="dim">5 repos</span>',body,'<span class="link">8 pull requests open</span><span class="sp"></span><span class="btn" style="opacity:.5">Send to claude</span>')+rail(True,' · 23'))

# ---------- 2-1 ask the agent
out='''<span class="u">&gt; show me the stack</span>

● humata · review_prepare(stack)
  <span class="dim">⎿ 3 layers · 5 repos · 4 key files, 9 to skim</span>

The Diff panel now shows the stack, bottom layer
first. Start at layer 1: two new tables and the
Terraform that sizes them. In layer 2 the file to
read closely is api/src/client.rs.

I only arranged it. Ask me to review it if you
want my findings on the lines.'''
L1P='Adds the accounts and entries tables'
stack1 = ly('', 'Whole stack', prs='5 repos · 23 files')+ly(1,'schema',pp=L1P,sel=True,prs='ledger · infra')+ly(2,'dual write',prs='api · ledger · sdk')+ly(3,'reads',prs='api · web · sdk')
files1 = repo('ledger','',pm(79,0))+fl('0042_accounts.sql',' migrations/',pm(41,0),sel=True,extra='<span class="badge">key</span> ')+fl('0043_entries.sql',' migrations/',pm(38,0))+repo('infra','',pm(18,0))+fl('ledger.tf',' db/',pm(18,0))
sql=[('+',1,'CREATE TABLE accounts ('),('+',2,'    id          uuid PRIMARY KEY,'),('+',3,'    owner_id    uuid NOT NULL,'),('+',4,'    currency    char(3) NOT NULL,'),('+',5,'    created_at  timestamptz NOT NULL DEFAULT now()'),('+',6,');')]
body=f'''<div class="list" style="flex:none"><div class="cap">STACK · 3 LAYERS</div>{stack1}<div class="sep"></div>{files1}</div>
<div class="fb">ledger · migrations/ <span style="color:#1d1d1f">0042_accounts.sql</span><span class="sp"></span>Open in editor</div>{lines(sql)}'''
strip='<div class="strip"><span class="b">●</span>claude arranged the stack and moved you to layer 1<span class="sp"></span><span class="link">Undo</span></div>'
page('2-1-ask-agent-to-show', sidebar()+session(T2,out)+panel(500,'<span class="dim">Layer 1 of 3</span>',body,'<span class="link">‹ Previous</span><span class="link">Next layer ›</span><span class="sp"></span><span class="btn">Looks good</span>',strip)+rail(True,' · 23'))

# ---------- 2-2 full screen, layer 1
sql2=sql+[(' ',7,''),('+',8,'CREATE TABLE entries ('),('+',9,'    id          bigserial PRIMARY KEY,'),('+',10,'    account_id  uuid NOT NULL REFERENCES accounts (id),'),('+',11,'    amount      numeric(20, 4) NOT NULL,'),('+',12,'    posted_at   timestamptz NOT NULL'),('+',13,');'),(' ',14,''),('+',15,'CREATE INDEX entries_posted_brin ON entries'),('+',16,'    USING brin (posted_at);')]
left=f'<div class="cap">STACK · 3 LAYERS</div>{stack1}<div class="sep"></div>{files1}<div class="f file dim">Repos this layer doesn\'t touch are hidden</div>'
code=f'<div class="fb">ledger · migrations/ <span style="color:#1d1d1f">0042_accounts.sql</span> <span class="badge">key</span><span class="sp"></span>Open in editor</div>{lines(sql2)}'
foot='<span class="link">‹ Previous layer</span><span class="link">Next layer ›</span><span class="dim">⌥⌘↑ ⌥⌘↓</span><span class="sp"></span><span class="dim">Layer 1 · 0 of 3 files looked at</span><span class="btn">Looks good</span>'
page('2-2-stack-full-screen', full('<span class="dim">Layer 1 of 3 · schema</span>',FS_BTNS,left,code,foot))

# ---------- 3-1 comment on a line, layer 2
L2P='Writes both tables in one transaction'
def stack2(s1='',s2='',s3='',sel=2,pp2=L2P,whole=False):
    return ly('', 'Whole stack', prs='5 repos · 23 files',sel=whole)+ly(1,'schema',st=s1,sel=sel==1,prs='ledger · infra')+ly(2,'dual write',st=s2,pp=pp2 if sel==2 else '',sel=sel==2,prs='api · ledger · sdk')+ly(3,'reads',st=s3,sel=sel==3,pp='Moves reads to entries' if sel==3 else '',prs='api · web · sdk')
def files2(c=0, cm='<span class="dim">▢ 2</span> '):
    return repo('api','',pm(143,24))+fl('client.rs',' src/',pm(62,21),ck=c,sel=True,extra='<span class="badge">key</span> '+cm)+fl('entry.rs',' src/',pm(81,3),ck=1)+repo('ledger','',pm(63,18))+fl('posting.rs',' src/',pm(63,18),ck=1)+repo('sdk','',pm(44,8))+fl('entry.ts',' src/',pm(44,8))
rs=[(' ',40,'    pub async fn post(&self, entry: Entry) -> Result<()> {'),('-',41,'        self.db.insert_legacy(&entry).await?;'),('+',41,'        let tx = self.db.begin().await?;'),('+',42,'        tx.insert_entry(&entry).await?;'),('+',43,'        tx.insert_legacy(&entry).await?;'),('+',44,'        tx.commit().await?;')]
rs_b=[(' ',45,'        self.metrics.posted.inc();'),(' ',46,'        Ok(())'),(' ',47,'    }'),(' ',48,''),('+',49,'    pub async fn post_many(&self, entries: &[Entry]) -> Result<()> {'),('+',50,'        for e in entries {'),('+',51,'            self.post(e.clone()).await?;'),('+',52,'        }')]
fb2='<div class="fb">api · src/ <span style="color:#1d1d1f">client.rs</span> <span class="badge">key</span><span class="sp"></span>Open in editor</div>'
c1=card('You','A failed commit is retried by the caller, and insert_legacy has no unique key. Make the legacy insert an upsert on entry.id.','<span class="badge">Draft</span><span class="sp"></span><span class="link">Edit</span>',cls='')
c2=f'<div class="card draft"><div class="top"><b>You</b><span class="badge">Draft</span><span class="dim">line 51</span></div>One transaction for the whole batch, not one per entry.<span style="border-left:1.5px solid #0a6cf0;margin-left:1px">&nbsp;</span><div class="acts"><span class="sp"></span><span class="dim">Esc to cancel</span><span class="link"><b>Save draft ⌘↩</b></span></div></div>'
code=fb2+lines(rs,'@@ -40,7 +40,12 @@ impl Client {')+c1+lines(rs_b)+c2
left=f'<div class="cap">STACK · 3 LAYERS</div>{stack2(s1=OK,s2="<span class=dim>2 drafts</span>")}<div class="sep"></div>{files2()}'
foot='<span class="link">‹ Previous layer</span><span class="link">Next layer ›</span><span class="sp"></span><span class="dim">2 drafts · 1 layer looks good</span><span class="btn primary">Send to claude</span>'
page('3-1-comment-on-a-line', full('<span class="dim">Layer 2 of 3 · dual write</span>',FS_BTNS,left,code,foot))

# ---------- 3-2 looks good per layer
L3files = repo('api','',pm(62,21))+fl('reads.rs',' src/ledger/',pm(62,21),ck=1,sel=True,extra='<span class="badge">key</span> ')+repo('web','',pm(71,30))+fl('LedgerTable.tsx',' src/ledger/',pm(71,30),ck=1)+fl('useEntries.ts',' src/ledger/',pm(24,9),ck=1)+repo('sdk','',pm(52,4))+fl('client.ts',' src/',pm(52,4),ck=1)+'<div class="f file dim">▸ 4 generated files · skim</div>'
rd=[(' ',18,'    pub async fn balance(&self, account: AccountId) -> Result<Decimal> {'),('-',19,'        let rows = self.db.legacy_entries(account).await?;'),('-',20,'        Ok(rows.iter().map(|r| r.amount).sum())'),('+',19,'        let sum = self.db'),('+',20,'            .entries_sum(account)'),('+',21,'            .await?;'),('+',22,'        Ok(sum.unwrap_or(Decimal::ZERO))'),(' ',23,'    }')]
left=f'<div class="cap">STACK · 3 LAYERS</div>{stack2(s1=OK,s2="<span class=b>3 drafts</span>",s3=OK,sel=3)}<div class="sep"></div>{L3files}'
code='<div class="fb">api · src/ledger/ <span style="color:#1d1d1f">reads.rs</span> <span class="badge">key</span><span class="sp"></span><span class="g">✓ You looked at this</span></div>'+lines(rd,'@@ -18,6 +18,8 @@ impl Reads {')
ov='''<div class="menu" style="right:14px;bottom:60px;width:330px"><div class="mcap">One message to claude:</div>
<div class="mi"><span class="b">▢</span>3 comments on layer 2 · dual write</div>
<div class="mi"><span class="g">✓</span>Layer 1 · schema looks good</div>
<div class="mi"><span class="g">✓</span>Layer 3 · reads looks good</div>
<div class="mcap" style="padding-top:8px">A layer that looks good is remembered. If its lines change later, the tick clears and you see only what changed.</div></div>'''
foot='<span class="link">‹ Previous layer</span><span class="link dim">Next layer ›</span><span class="sp"></span><span class="btn ok">✓ Looks good</span><span class="btn primary">Send to claude ▴</span>'
page('3-2-looks-good-per-layer', full('<span class="dim">Layer 3 of 3 · reads</span>',FS_BTNS,left,code,foot,ov))

# ---------- 3-3 new agent tab menu
left=f'<div class="cap">STACK · 3 LAYERS</div>{stack2(s1=OK,s2="<span class=b>3 drafts</span>",s3=OK)}<div class="sep"></div>{files2()}'
code=fb2+lines(rs,'@@ -40,7 +40,12 @@ impl Client {')+c1+lines(rs_b)
ov=f'''<div class="menu" style="right:150px;top:54px;width:340px"><div class="mcap">Opens a new tab in this session, beside the diff.<br>It starts with no history. Pick the model inside the agent.</div>
<div class="mi"><span class="dim">&gt;_</span>Terminal<span class="sp"></span><span class="dim">⌘T</span></div><div class="sep"></div>
<div class="mi">{dot(GREEN)}claude</div><div class="mi on">{dot(GREEN)}copilot<span class="sp"></span><span class="dim">last used</span></div><div class="mi dim">{dot(GREY)}codex · not installed</div></div>'''
btns='<span class="btn" style="background:#e4e4ea">New agent tab ⌄</span><span class="btn">Exit full screen</span>'
page('3-3-new-agent-tab-menu', full('<span class="dim">Layer 2 of 3 · dual write</span>',btns,left,code,foot.replace('btn ok','btn').replace('✓ Looks good','Looks good').replace(' ▴',''),ov))

# ---------- 3-4 fresh agent beside diff
out='''<span class="dim">~/src (ledger-entries) $ copilot</span>
<span class="dim">New agent. No history from this session.</span>
<span class="u">&gt; Look at layer 2 of the stack in the Diff panel.
  Don't edit files. Leave what you find on the lines.</span>

● humata · review_context
  <span class="dim">⎿ Layer 2 · dual write · api, ledger, sdk</span>
● Read(api/src/client.rs)
● Read(ledger/src/posting.rs)
● Grep("insert_legacy")
  <span class="dim">⎿ 3 matches in 2 files</span>

The transaction closes the crash gap. One thing
is still open: posting.rs:88 reads the balance
outside the transaction, so two posts can both
pass the overdraft check.

● humata · review_note(ledger/src/posting.rs:88)

<span class="o">✱ Checking the sdk types… (38s · esc to interrupt)</span>'''
T3=[('claude',ORANGE,False),('zsh',None,False),('copilot',GREEN,True)]
po=[(' ',86,'    let tx = db.begin().await?;'),('+',87,'    tx.insert_entry(&e).await?;'),(' ',88,'    let bal = db.balance(e.account).await?;'),(' ',89,'    ensure!(bal >= Decimal::ZERO, Overdrawn);')]
note=card('copilot','The balance is read on db, not tx, so two posts can both pass the overdraft check. Read it through tx.','<span class="badge or">Note</span><span class="dim">line 88</span>',acts='<span class="link"><b>Send to claude</b></span><span class="link">Edit</span><span class="link dim">Drop</span>')
filesp=repo('api','',pm(143,24))+fl('client.rs',' src/',pm(62,21),extra='<span class="dim">▢ 3</span> ')+repo('ledger','',pm(63,18))+fl('posting.rs',' src/',pm(63,18),sel=True,extra='<span class="o">▢ 1</span> ')
body=f'''<div class="list" style="flex:none"><div class="cap">STACK · 3 LAYERS</div>{stack2(s1=OK,s2="<span class=b>3 drafts · 1 note</span>",s3=OK,pp2='')}<div class="sep"></div>{filesp}</div>
<div class="fb">ledger · src/ <span style="color:#1d1d1f">posting.rs</span><span class="sp"></span>Open in editor</div>{lines(po,'@@ -84,6 +84,7 @@ pub async fn post(')}{note}'''
page('3-4-fresh-agent-beside-diff', sidebar(ORANGE)+session(T3,out,right='<span class="tab dim" style="border:0">2 agents</span>')+panel(520,'<span class="dim">Layer 2 of 3</span>',body,'<span class="dim">3 drafts · 1 note · 2 layers look good</span><span class="sp"></span><span class="btn primary">Send to claude</span>')+rail(True,' · 23'))

# ---------- 4-1 agent gets it and applies
out='''<span class="u">&gt; From the Diff panel · 4 comments, 2 layers look good
  Layer 1 · schema: looks good.   Layer 3 · reads: looks good.
  Layer 2 · dual write:
   api/src/client.rs:43     Make the legacy insert an upsert on entry.id.
   api/src/client.rs:51     One transaction for the whole batch.
   api/src/client.rs:60     Name the timeout.
   ledger/src/posting.rs:88 Read the balance through tx.  (from copilot)</span>

● Update(api/src/client.rs)
  <span class="dim">⎿ Updated with 19 additions and 8 removals</span>
● Update(ledger/src/posting.rs)
  <span class="dim">⎿ Updated with 2 additions and 2 removals</span>
● Bash(bazel test //api/... //ledger/...)
  <span class="g">⎿ 33 tests pass</span>
● Bash(stack submit --all)
  <span class="dim">⎿ updated acme/api#482, acme/ledger#92 · rebased layer 3</span>
● humata · comment_resolve × 4

All four are done, each with a reply on its line.
Layers 1 and 3 didn't change.'''
stack4 = ly('', 'Whole stack', prs='5 repos · 23 files',sel=True)+ly(1,'schema',st=OK,prs='ledger · infra')+ly(2,'dual write',st='<span class="o">● Changed since you looked</span>',prs='3 repos')+ly(3,'reads',st=OK,prs='api · web · sdk')
body=f'''<div class="list" style="flex:1"><div class="cap">STACK · 3 LAYERS</div>{stack4}<div class="sep"></div>
{repo('api','against main',pm(216,45))}{fl('client.rs',' src/',pm(73,21),ck=2,extra='<span class="g">✓ 3 done</span> ')}{fl('entry.rs',' src/',pm(81,3),ck=1)}{fl('reads.rs',' src/ledger/',pm(62,21),ck=1)}
{repo('infra','against main',pm(18,0))}{fl('ledger.tf',' db/',pm(18,0),ck=1)}
{repo('ledger','against release/2.4',pm(142,18))}{fl('0042_accounts.sql',' migrations/',pm(41,0),ck=1)}{fl('0043_entries.sql',' migrations/',pm(38,0),ck=1)}{fl('posting.rs',' src/',pm(63,18),ck=2,extra='<span class="g">✓ 1 done</span> ')}
{repo('sdk','against main',pm(96,12))}{fl('entry.ts',' src/',pm(44,8),ck=1)}{fl('client.ts',' src/',pm(52,4),ck=1)}
{repo('web','against main',pm(71,30))}{fl('LedgerTable.tsx',' src/ledger/',pm(71,30),ck=1)}</div>'''
seg='<span class="seg"><span>All changes</span><span class="on">Since I last looked · 2 files</span></span>'
page('4-1-agent-applies-comments', sidebar(GREEN,'<span class="sp"></span><span style="font-weight:400;font-size:12px">2 changed</span>')+session(T3[:2]+[('copilot',GREY,False)],out)+panel(500,'',body,'<span class="dim">4 comments done · 2 files changed</span><span class="sp"></span><span class="link">Show what changed</span>')+rail(True,' · 2'))

# ---------- 4-2 manual: since I last looked
rs2=[(' ',41,'        let tx = self.db.begin().await?;'),(' ',42,'        tx.insert_entry(&entry).await?;'),('-',43,'        tx.insert_legacy(&entry).await?;'),('+',43,'        tx.upsert_legacy(&entry).await?;'),(' ',44,'        tx.commit().await?;')]
rs3=[(' ',49,'    pub async fn post_many(&self, entries: &[Entry]) -> Result<()> {'),('-',50,'        for e in entries {'),('-',51,'            self.post(e.clone()).await?;'),('-',52,'        }'),('+',50,'        let tx = self.db.begin().await?;'),('+',51,'        for e in entries {'),('+',52,'            tx.insert_entry(e).await?;'),('+',53,'            tx.upsert_legacy(e).await?;'),('+',54,'        }'),('+',55,'        tx.commit().await?;')]
d1=card('You','Make the legacy insert an upsert on entry.id.','<span class="badge gr">Done by claude</span><span class="dim">line 43</span><span class="sp"></span><span class="link">Reopen</span>',cls='done',reply='<b>claude</b> &nbsp;upsert_legacy keys on entry.id, so a retried commit writes the row once.')
d2=card('You','One transaction for the whole batch, not one per entry.','<span class="badge gr">Done by claude</span><span class="dim">line 51</span><span class="sp"></span><span class="link">Reopen</span>',cls='done',reply='<b>claude</b> &nbsp;post_many opens one transaction and commits once.')
left=f'''<div class="cap">2 FILES CHANGED SINCE YOU LOOKED</div>
{repo('api','against main',pm(11,3))}{fl('client.rs',' src/',pm(11,3),ck=2,sel=True,extra='<span class="g">✓ 3 done</span> ')}
{repo('ledger','against release/2.4',pm(2,2))}{fl('posting.rs',' src/',pm(2,2),ck=2,extra='<span class="g">✓ 1 done</span> ')}
<div class="sep"></div><div class="f dim">infra, sdk, web · nothing changed since you looked</div>
<div class="sep"></div><div class="cap">STACK · 3 LAYERS</div>{stack4.replace('ly sel','ly')}'''
code='<div class="fb">api · src/ <span style="color:#1d1d1f">client.rs</span><span class="dim">· changes since you looked, 12 min ago</span><span class="sp"></span>Open in editor</div>'+lines(rs2,'@@ -41,4 +41,4 @@ impl Client {')+d1+lines(rs3,'@@ -49,4 +49,7 @@')+d2
foot='<span class="dim">File 1 of 2</span><span class="link">Next file ›</span><span class="sp"></span><span class="dim">4 comments done · 0 open</span><span class="btn">Looks good</span>'
page('4-2-manual-diff-since-last-look', full(seg,FS_BTNS,left,code,foot))

# ---------- 4-3 all good
stack5 = ly('', 'Whole stack', prs='5 repos · 23 files',sel=True,st=OK)+ly(1,'schema',st=OK,prs='ledger#91 · infra#40')+ly(2,'dual write',st=OK,prs='api#482 · ledger#92 · sdk#17')+ly(3,'reads',st=OK,prs='api#483 · web#210 · sdk#18')
left=f'''<div class="cap">STACK · 3 LAYERS</div>{stack5}<div class="sep"></div>
{repo('api','against main',pm(216,45))}{fl('client.rs',' src/',pm(73,21),ck=1,sel=True)}{fl('entry.rs',' src/',pm(81,3),ck=1)}{fl('reads.rs',' src/ledger/',pm(62,21),ck=1)}
{repo('infra','against main',pm(18,0))}{fl('ledger.tf',' db/',pm(18,0),ck=1)}
{repo('ledger','against release/2.4',pm(142,18))}{fl('0042_accounts.sql',' migrations/',pm(41,0),ck=1)}{fl('0043_entries.sql',' migrations/',pm(38,0),ck=1)}{fl('posting.rs',' src/',pm(63,18),ck=1)}
{repo('sdk','against main',pm(96,12))}{fl('entry.ts',' src/',pm(44,8),ck=1)}{fl('client.ts',' src/',pm(52,4),ck=1)}
{repo('web','against main',pm(71,30))}{fl('LedgerTable.tsx',' src/ledger/',pm(71,30),ck=1)}'''
code='<div class="fb">api · src/ <span style="color:#1d1d1f">client.rs</span><span class="sp"></span><span class="g">✓ You looked at this</span></div>'+lines([(' ',40,'    pub async fn post(&self, entry: Entry) -> Result<()> {'),('-',41,'        self.db.insert_legacy(&entry).await?;'),('+',41,'        let tx = self.db.begin().await?;'),('+',42,'        tx.insert_entry(&entry).await?;'),('+',43,'        tx.upsert_legacy(&entry).await?;'),('+',44,'        tx.commit().await?;')],'@@ -40,7 +40,12 @@ impl Client {')+'<div class="card done" style="padding:7px 12px"><span class="g">✓</span> 3 comments done on this file &nbsp;<span class="link">Show</span></div>'
seg2='<span class="seg"><span class="on">All changes</span><span>Since I last looked · 0 files</span></span>'
foot='<span class="g">✓ All 3 layers look good · 4 comments done</span><span class="sp"></span><span class="dim">8 pull requests are up to date</span><span class="btn primary">Open the pull requests</span>'
page('4-3-everything-looks-good', full(seg2,FS_BTNS,left,code,foot))
