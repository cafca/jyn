import 'package:flutter/material.dart';
import 'package:jyn/src/rust/api/lifecycle.dart';
import 'package:jyn/src/rust/frb_generated.dart';

Future<void> main() async {
  await RustLib.init();
  await startNode();
  runApp(const JynApp());
}

class JynApp extends StatelessWidget {
  const JynApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      title: 'jyn',
      home: Scaffold(body: Center(child: Text('jyn — node running'))),
    );
  }
}
