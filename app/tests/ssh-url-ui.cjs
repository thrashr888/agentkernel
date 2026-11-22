(async()=>{
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const { transformWithOxc } = await import('vite');
const { test } = require('node:test');
const file = fs.readFileSync('src/pages/SandboxDetail.tsx','utf8');
const component = file.slice(file.indexOf('function SandboxSshUrl('),file.indexOf('function parsePort'));
const compiled = (await transformWithOxc(component,'fixture.tsx',{jsx:{runtime:'classic'}})).code;
const old='ssh://sandbox@127.0.0.1:2200', fresh='ssh://sandbox@127.0.0.1:4400';
function fixture(lookup, {clipboard=true, revalidate=()=>Promise.resolve(fresh)}={}) {
  const writes=[], successes=[], errors=[], invalidated=[], cache=[], options=[];
  let cleanup;
  const make=(type,props,...children)=>({type,props:props??{},children});
  const context={React:{createElement:make},Button:'Button',Copy:'Icon',
    Blob, ClipboardItem:clipboard ? class { constructor(entries){this.address=entries['text/plain'];} } : undefined,
    navigator:{clipboard:{write(items){writes.push(items[0].address);return items[0].address.then(()=>undefined);}}},
    useRef(current){return {current};},useEffect(effect){cleanup=effect();},
    useQueryClient:()=>({setQueryData(key,url){cache.push([key,url]);},invalidateQueries(x){invalidated.push(x);}}),
    useQuery(o){options.push(o); return lookup;},
    useMutation(o){return {isPending:false, async mutateAsync(){try { const result=await o.mutationFn(); o.onSuccess(result); return result; } catch(e) {o.onError();throw e;}}};},
    api:{getSandboxSshUrl:revalidate},toast:{success(s){successes.push(s);},error(s){errors.push(s);}},
  };
  vm.createContext(context);vm.runInContext(compiled,context);
  const tree=context.SandboxSshUrl({name:'demo',scope:'selected-server+uuid'});
  return {tree,copy:()=>tree.children.find(x=>x.type==='Button').props.onClick(),cleanup,writes,successes,errors,invalidated,cache,options};
}
test('renders verified URL directly, scopes query and stages freshly looked-up bytes immediately on gesture', async()=>{
  const f=fixture({data:old});assert.equal(f.tree.children[0].children[0],old);
  assert.deepEqual(Array.from(f.options[0].queryKey),['sandbox-ssh-url','selected-server+uuid','demo']);
  assert.equal(f.options[0].refetchInterval, 5000);
  f.copy();assert.equal(f.writes.length,1);assert.equal(await (await f.writes[0]).text(),fresh);
  assert.equal(f.cache[0][1],fresh);assert.equal(f.successes.length,1);
});
test('never renders cached data while refreshing or after lookup failure',()=>{
  for(const state of [{data:old,isFetching:true},{data:old,isError:true,error:'No verified address'}]){
    const f=fixture(state);assert.equal(f.tree.type,'span');assert.notEqual(f.tree.children[0],old);
  }
});
test('fresh lookup failure never copies stale bytes and invalidates displayed query',async()=>{
  const f=fixture({data:old},{revalidate:()=>Promise.reject(new Error('stopped'))});
  f.copy();await assert.rejects(f.writes[0]);await Promise.resolve();assert.equal(f.invalidated.length,1);
  assert.equal(f.successes.length,0);assert.equal(f.errors.length,1);
});
test('selection change blocks clipboard value from an old request',async()=>{
  let resolve;
  const f=fixture({data:old},{revalidate:()=>new Promise(r=>{resolve=r;})});
  f.copy();f.cleanup();resolve(fresh);await assert.rejects(f.writes[0],/selection changed/i);
  assert.equal(f.cache.length,0);
});
test('unsupported WebKit clipboard keeps selectable text',()=>{
  const f=fixture({data:old},{clipboard:false});f.copy();
  assert.match(f.tree.children[0].props.className,/select-text/);
  assert.equal(f.writes.length,0);assert.match(f.errors[0],/manually/);
});

test('stopped sandbox, remote server, and other backends hide the endpoint and copy control',()=>{
  const guard=file.match(/\{(isRunning && localSshScope && sandbox\.backend\.toLowerCase\(\) === "docker") && \(/);
  assert.ok(guard);
  const visible=(isRunning,localSshScope,backend) =>
    Boolean(vm.runInNewContext(guard[1], {isRunning,localSshScope,sandbox:{backend}}));
  assert.equal(visible(true,'local','Docker'),true);
  assert.equal(visible(false,'local','Docker'),false);
  assert.equal(visible(true,null,'Docker'),false);
  assert.equal(visible(true,'local','Podman'),false);
});
})().catch(e=>{console.error(e);process.exitCode=1;});
