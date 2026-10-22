"""Private acceptance preparation only: mocked HTTP, invented private configs."""
import importlib.util
import io
import email.message
import urllib.error
import json
import os
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import urllib.request

SPEC=importlib.util.spec_from_file_location('http_acceptance',Path(__file__).with_name('http-acceptance.py'))
MODULE=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PrivateAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory(prefix='ka-http-client-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root=Path(self.temporary.name).resolve()
        self.config=self.root/'private.json'
        self.document={'hostname':'book.example.ts.net','login':'owner@example.com','socket':str(self.root/'proxy.sock')}
        self.write_config()
        self.old_origin,self.old_private=MODULE.BASE_ORIGIN,MODULE.PRIVATE_MODE
        self.addCleanup(lambda:setattr(MODULE,'BASE_ORIGIN',self.old_origin))
        self.addCleanup(lambda:setattr(MODULE,'PRIVATE_MODE',self.old_private))

    def write_config(self,document=None):
        self.config.write_text(json.dumps(document if document is not None else self.document))
        self.config.chmod(0o600)

    def test_success_response_header_names_are_normalized_without_changing_values(self):
        headers = email.message.Message()
        headers['Cache-Control'] = 'no-store'
        headers['cOnTeNt-TyPe'] = 'application/json'
        headers['X-Content-Type-Options'] = 'nosniff'
        response = mock.MagicMock()
        response.__enter__.return_value = response
        response.status = 200
        response.headers = headers
        response.read.return_value = b'{"selection_required":true}'
        with mock.patch.object(MODULE, 'open_request', return_value=response) as opened:
            status, normalized, body = MODULE.get('/api/session')
        self.assertEqual(status, 200)
        self.assertEqual(normalized, {'cache-control':'no-store', 'content-type':'application/json', 'x-content-type-options':'nosniff'})
        self.assertEqual(body, b'{"selection_required":true}')
        self.assertEqual(opened.call_args.args[0].full_url, MODULE.BASE_ORIGIN + '/api/session')

    def test_http_error_header_names_are_normalized_on_the_error_path(self):
        headers = email.message.Message()
        headers['Cache-Control'] = 'no-store'
        headers['Content-TYPE'] = 'application/json'
        failure = urllib.error.HTTPError(MODULE.BASE_ORIGIN + '/api/session', 403, 'Forbidden', headers, io.BytesIO(b'{"error":"denied"}'))
        with mock.patch.object(MODULE, 'open_request', side_effect=failure):
            status, normalized, body = MODULE.get('/api/session')
        self.assertEqual(status, 403)
        self.assertEqual(normalized, {'cache-control':'no-store', 'content-type':'application/json'})
        self.assertEqual(body, b'{"error":"denied"}')

    def test_private_config_derives_only_validated_https_origin_without_network(self):
        with mock.patch.object(MODULE.urllib.request,'build_opener') as opener:
            self.assertEqual(MODULE.load_private_origin(self.config),'https://book.example.ts.net')
            opener.assert_not_called()

    def test_public_oversized_symlink_hardlink_and_noncanonical_configs_refuse(self):
        self.config.chmod(0o644)
        with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        self.config.chmod(0o600)
        link=self.root/'link.json';link.symlink_to(self.config)
        with self.assertRaises(ValueError):MODULE.load_private_origin(link)
        hardlink=self.root/'hardlink.json';os.link(self.config,hardlink)
        with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        hardlink.unlink()
        with self.assertRaises(ValueError):MODULE.load_private_origin(Path('relative.json'))
        self.config.write_bytes(b'x'*16385)
        with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)

    def test_parent_source_and_unsafe_document_fields_refuse(self):
        self.root.chmod(0o755)
        try:
            with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        finally:self.root.chmod(0o700)
        with mock.patch.object(MODULE,'SOURCE',self.root):
            with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        for field,value in [('hostname','https://book.example.ts.net'),('hostname','evil.example'),('hostname','BOOK.example.ts.net'),('socket','relative.sock'),('socket','/tmp/../socket'),('login','not-a-login')]:
            document=dict(self.document);document[field]=value;self.write_config(document)
            with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        self.write_config(dict(self.document,extra='refuse'))
        with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)
        self.config.write_text('{"hostname":"book.example.ts.net","hostname":"other.example.ts.net","login":"owner@example.com","socket":"/private/tmp/x"}')
        with self.assertRaises(ValueError):MODULE.load_private_origin(self.config)

    def test_fixed_origin_opener_disables_proxies_redirects_and_arbitrary_urls(self):
        MODULE.BASE_ORIGIN='https://book.example.ts.net'
        request=urllib.request.Request(MODULE.BASE_ORIGIN+'/api/session/selection',data=b'{}',method='POST',headers={'Origin':MODULE.BASE_ORIGIN})
        with mock.patch.object(MODULE.urllib.request,'build_opener') as builder:
            MODULE.open_request(request,timeout=30)
            handlers=builder.call_args.args
            self.assertIsInstance(handlers[0],urllib.request.ProxyHandler)
            self.assertEqual(handlers[0].proxies,{})
            self.assertIsInstance(handlers[1],MODULE.NoRedirect)
            builder.return_value.open.assert_called_once_with(request,timeout=30)
        for url in ['https://evil.example/api/session','http://127.0.0.1:3000/api/session','https://user@book.example.ts.net/api/session']:
            with self.assertRaises(ValueError):MODULE.open_request(url)
        self.assertIsNone(MODULE.NoRedirect().redirect_request(None,None,302,'Found',{},'https://evil.example'))

    def test_fixture_proof_precedes_even_book_selection_write(self):
        with mock.patch.object(sys,'argv',['http-acceptance.py','--fixture-token','a'*32,'--choose-book','--writes']),mock.patch.object(MODULE,'get',return_value=(200,{},b'kAccounting-Extra-01 wrong fixture')),mock.patch.object(MODULE,'open_request') as write:
            with self.assertRaises(ValueError):MODULE.main()
            write.assert_not_called()
        MODULE.require_fixture_marker(b'<!-- kAccounting-acceptance:'+b'a'*32+b' -->\nrest','a'*32)

    def test_invalid_private_config_exits_before_any_http_request(self):
        self.config.chmod(0o644)
        with mock.patch.object(sys,'argv',['http-acceptance.py','--fixture-token','a'*32,'--private-config',str(self.config)]),mock.patch.object(MODULE,'get') as request,mock.patch.object(sys,'stderr'):
            with self.assertRaises(SystemExit) as exit:
                MODULE.main()
            self.assertEqual(exit.exception.code,2)
            request.assert_not_called()

    def test_private_wrong_host_can_be_404_but_origin_fetch_site_remain_403(self):
        MODULE.PRIVATE_MODE=True
        MODULE.forbidden_status({'Host':'evil.example'},404)
        MODULE.forbidden_status({'Host':'evil.example'},403)
        for headers in [{'Origin':'https://evil.example'},{'Sec-Fetch-Site':'cross-site'}]:
            MODULE.forbidden_status(headers,403)
            with self.assertRaises(AssertionError):MODULE.forbidden_status(headers,404)
        MODULE.PRIVATE_MODE=False
        with self.assertRaises(AssertionError):MODULE.forbidden_status({'Host':'evil.example'},404)

    def test_capability_file_is_bound_to_private_exact_fixture_parent_and_nonce(self):
        token='a'*32;root=self.root/('ka-http-acceptance-'+token);root.mkdir(mode=0o700)
        path=root/'transfer-capabilities.json';path.write_text(json.dumps({'fixture_token':token}));path.chmod(0o600)
        self.assertEqual(MODULE.capability_path(path,token),path)
        self.assertEqual(MODULE.read_capabilities(path),{'fixture_token':token})
        with self.assertRaises(ValueError):MODULE.capability_path(path,'b'*32)
        with self.assertRaises(ValueError):MODULE.capability_path(self.config,token)
        path.chmod(0o644)
        with self.assertRaises(ValueError):MODULE.read_capabilities(path)


if __name__=='__main__':unittest.main()
